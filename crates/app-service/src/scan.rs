//! Scan orchestration lives here so host adapters only consume update DTOs.

use std::path::Path;
use std::time::Duration;

use photo_catalog::CatalogError;
use photo_core::FolderPolicyEngine;
use photo_indexer::{IndexEvent, Indexer, ScanRequest};

use crate::service::{ScanOwner, SelectionToken, ServiceState};
use crate::{AppService, AppServiceError, BootstrapState, WallUpdate};

impl AppService {
    /// Starts a cancellable scan after persisting the active selection. The catalog lock is
    /// released before any source read or event wait.
    pub async fn start_scan(&self, folder: &Path) -> Result<BootstrapState, AppServiceError> {
        let (bootstrap, selection) = self.select_recent(folder)?;
        self.start_selected_scan(selection).await?;
        Ok(bootstrap)
    }

    pub(crate) async fn start_selected_scan(
        &self,
        selection_token: SelectionToken,
    ) -> Result<(), AppServiceError> {
        let (selection_root, library_root, owner) = {
            let mut state = self.state()?;
            let stored = state.libraries.catalog().load_app_state()?;
            let selection = stored
                .active_selection
                .ok_or(AppServiceError::StatePoisoned)?;
            if state.selection_epoch != selection_token.epoch
                || selection.library_id != selection_token.library_id
            {
                return Ok(());
            }
            let library = state
                .libraries
                .catalog()
                .find_library(selection.library_id)?
                .ok_or(AppServiceError::StatePoisoned)?;
            let root = library
                .canonical_root_key
                .to_path_buf()
                .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
            let selected = selection
                .relative_folder
                .to_path_buf()
                .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
            let group_id = state
                .libraries
                .catalog()
                .folder_group_for_path(selection.library_id, &selection.relative_folder)?
                .ok_or(AppServiceError::StatePoisoned)?;
            if group_id != selection_token.group_id || state.active_scan.is_some() {
                return Ok(());
            }
            let generation = state
                .libraries
                .catalog_mut()
                .begin_generation_for_group(selection.library_id, group_id)?;
            let owner = ScanOwner {
                selection: selection_token,
                generation,
            };
            state.active_scan = Some(owner);
            (root.clone().join(&selected), root, owner)
        };
        self.wake_derivative_workers();
        let policy = match FolderPolicyEngine::new(Vec::new()) {
            Ok(policy) => policy,
            Err(error) => {
                if let Ok(mut state) = self.state.lock()
                    && state.active_scan == Some(owner)
                {
                    state.active_scan = None;
                    state.active_cancel = None;
                }
                self.wake_derivative_workers();
                return Err(AppServiceError::LibrarySetup(std::io::Error::other(
                    error.to_string(),
                )));
            }
        };
        let indexer =
            Indexer::with_scheduler(self.metadata_reader.clone(), policy, self.scheduler.clone());
        let handle = match indexer.start(
            ScanRequest::new(selection_root.clone())
                .for_library(owner.selection.library_id)
                .roots(library_root, selection_root)
                .for_folder_group(owner.selection.group_id),
        ) {
            Ok(handle) => handle,
            Err(photo_indexer::IndexError::RootUnavailable(_)) => {
                let mut state = self.state()?;
                if state.active_scan != Some(owner) {
                    return Ok(());
                }
                if let Err(error) = mark_unavailable_selection(&mut state, owner) {
                    state.active_scan = None;
                    state.active_cancel = None;
                    drop(state);
                    self.wake_derivative_workers();
                    return Err(error);
                }
                state.active_scan = None;
                state.active_cancel = None;
                let _ = self.updates.send(WallUpdate::SourceUnavailable {
                    selection_id: owner.selection.selection_id(),
                    source_id: owner
                        .selection
                        .library_id
                        .as_uuid()
                        .hyphenated()
                        .to_string(),
                });
                drop(state);
                self.wake_derivative_workers();
                return Ok(());
            }
            Err(error) => {
                if let Ok(mut state) = self.state.lock()
                    && state.active_scan == Some(owner)
                {
                    state.active_scan = None;
                    state.active_cancel = None;
                }
                self.wake_derivative_workers();
                return Err(AppServiceError::LibrarySetup(std::io::Error::other(
                    error.to_string(),
                )));
            }
        };
        let cancel = handle.cancellation_sender();
        if let Ok(mut state) = self.state.lock() {
            if state.active_scan != Some(owner) {
                let _ = cancel.send(true);
                return Ok(());
            }
            state.active_cancel = Some(cancel);
        }
        let service = self.clone();
        tokio::spawn(async move {
            service.drain_scan(handle, owner).await;
        });
        Ok(())
    }

    pub(crate) async fn reconcile_existing(&self) {
        let details = (|| -> Result<_, AppServiceError> {
            let mut state = self.state()?;
            if state.active_scan.is_some() {
                return Err(AppServiceError::StatePoisoned);
            }
            let selection = state
                .libraries
                .catalog()
                .load_app_state()?
                .active_selection
                .ok_or(AppServiceError::StatePoisoned)?;
            let library = state
                .libraries
                .catalog()
                .find_library(selection.library_id)?
                .ok_or(AppServiceError::StatePoisoned)?;
            let root = library
                .canonical_root_key
                .to_path_buf()
                .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
            let selected = selection
                .relative_folder
                .to_path_buf()
                .map_err(|error| CatalogError::InvalidData(error.to_string()))?;
            let group = state
                .libraries
                .catalog()
                .folder_group_for_path(selection.library_id, &selection.relative_folder)?
                .ok_or(AppServiceError::StatePoisoned)?;
            let token = SelectionToken {
                library_id: selection.library_id,
                group_id: group,
                epoch: state.selection_epoch,
            };
            let generation = state
                .libraries
                .catalog_mut()
                .begin_generation_for_group(selection.library_id, group)?;
            let owner = ScanOwner {
                selection: token,
                generation,
            };
            state.active_scan = Some(owner);
            Ok((root.clone().join(selected), root, owner))
        })();
        let Ok((selection_root, library_root, owner)) = details else {
            return;
        };
        self.wake_derivative_workers();
        if !selection_root.is_dir() {
            if let Ok(mut state) = self.state.lock()
                && state.active_scan == Some(owner)
            {
                let _ = mark_unavailable_selection(&mut state, owner);
                state.active_scan = None;
                state.active_cancel = None;
                let _ = self.updates.send(WallUpdate::SourceUnavailable {
                    selection_id: owner.selection.selection_id(),
                    source_id: owner
                        .selection
                        .library_id
                        .as_uuid()
                        .hyphenated()
                        .to_string(),
                });
            }
            self.wake_derivative_workers();
            return;
        }
        let indexer = match FolderPolicyEngine::new(Vec::new()) {
            Ok(policy) => Indexer::with_scheduler(
                self.metadata_reader.clone(),
                policy,
                self.scheduler.clone(),
            ),
            Err(_) => {
                if let Ok(mut state) = self.state.lock()
                    && state.active_scan == Some(owner)
                {
                    state.active_scan = None;
                    state.active_cancel = None;
                }
                self.wake_derivative_workers();
                return;
            }
        };
        let handle = match indexer.start(
            ScanRequest::new(selection_root.clone())
                .for_library(owner.selection.library_id)
                .roots(library_root, selection_root)
                .for_folder_group(owner.selection.group_id),
        ) {
            Ok(handle) => handle,
            Err(photo_indexer::IndexError::RootUnavailable(_)) => {
                if let Ok(mut state) = self.state.lock() {
                    if state.active_scan != Some(owner) {
                        return;
                    }
                    let _ = mark_unavailable_selection(&mut state, owner);
                    state.active_scan = None;
                    state.active_cancel = None;
                    let _ = self.updates.send(WallUpdate::SourceUnavailable {
                        selection_id: owner.selection.selection_id(),
                        source_id: owner
                            .selection
                            .library_id
                            .as_uuid()
                            .hyphenated()
                            .to_string(),
                    });
                    drop(state);
                    self.wake_derivative_workers();
                }
                return;
            }
            Err(_) => {
                if let Ok(mut state) = self.state.lock()
                    && state.active_scan == Some(owner)
                {
                    state.active_scan = None;
                    state.active_cancel = None;
                }
                self.wake_derivative_workers();
                return;
            }
        };
        if let Ok(mut state) = self.state.lock() {
            if state.active_scan != Some(owner) {
                let _ = handle.cancellation_sender().send(true);
                return;
            }
            state.active_cancel = Some(handle.cancellation_sender());
        }
        self.drain_scan(handle, owner).await;
    }

    async fn drain_scan(&self, mut handle: photo_indexer::ScanHandle, owner: ScanOwner) {
        let mut writer_events = Vec::new();
        let mut last = photo_indexer::ScanProgress {
            stage: photo_indexer::ScanStage::Discovering,
            discovered: 0,
            shaped: 0,
            enriched: 0,
            total: None,
        };
        while let Some(first) = handle.events.recv().await {
            writer_events.push(first);
            let deadline = tokio::time::sleep(Duration::from_millis(50));
            tokio::pin!(deadline);
            while writer_events.len() < 200 {
                tokio::select! {
                    _ = &mut deadline => break,
                    event = handle.events.recv() => match event {
                        Some(event) => writer_events.push(event),
                        None => break,
                    }
                }
            }
            for event in &writer_events {
                if let IndexEvent::Progress(progress) = event {
                    last = *progress;
                }
            }
            let shaped = writer_events
                .iter()
                .filter_map(|event| match event {
                    IndexEvent::ShapeReady { asset_id, .. }
                    | IndexEvent::ShapeFallback { asset_id, .. } => Some(*asset_id),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let mut state = match self.state.lock() {
                Ok(state) => state,
                Err(_) => return,
            };
            if state.active_scan != Some(owner) {
                return;
            }
            let mut writer = photo_indexer::CatalogWriter::new(
                state.libraries.catalog_mut(),
                owner.selection.library_id,
                owner.generation,
            );
            if writer.apply_batch(&writer_events).is_err() {
                state.active_scan = None;
                state.active_cancel = None;
                drop(state);
                self.wake_derivative_workers();
                return;
            }
            if !shaped.is_empty() {
                let assets = state
                    .libraries
                    .catalog()
                    .wall_records_for_assets(owner.selection.group_id, &shaped)
                    .ok()
                    .and_then(|records| {
                        crate::service::wall_assets_with_derivatives(
                            state.libraries.catalog(),
                            &records,
                            crate::OrderState::Provisional,
                        )
                        .ok()
                    })
                    .unwrap_or_default();
                if !assets.is_empty() {
                    let _ = self.updates.send(WallUpdate::CatalogBatch {
                        selection_id: owner.selection.selection_id(),
                        assets,
                        order_state: crate::OrderState::Provisional,
                        generation: owner.generation,
                        progress: progress_dto(last),
                    });
                }
            }
            if last.discovered > 0 {
                let _ = self.updates.send(WallUpdate::Progress {
                    selection_id: owner.selection.selection_id(),
                    generation: owner.generation,
                    progress: progress_dto(last),
                });
            }
            drop(state);
            writer_events.clear();
        }
        let successful = handle
            .join()
            .await
            .map(|summary| !summary.cancelled)
            .unwrap_or(false);
        let successful_selection = if let Ok(mut state) = self.state.lock() {
            if state.active_scan != Some(owner) {
                return;
            }
            state.active_scan = None;
            state.active_cancel = None;
            if successful {
                let completed = state
                    .libraries
                    .catalog_mut()
                    .complete_generation_for_group(
                        owner.selection.library_id,
                        owner.selection.group_id,
                        owner.generation,
                    )
                    .is_ok();
                if !completed {
                    state.active_scan = None;
                    state.active_cancel = None;
                    drop(state);
                    self.wake_derivative_workers();
                    return;
                }
                let _ = self.updates.send(WallUpdate::MetadataSettled {
                    selection_id: owner.selection.selection_id(),
                    source_id: owner
                        .selection
                        .library_id
                        .as_uuid()
                        .hyphenated()
                        .to_string(),
                    generation: owner.generation,
                });
                Some(())
            } else {
                None
            }
        } else {
            return;
        };
        self.wake_derivative_workers();
        #[cfg(test)]
        self.notify_scan_completion_wake_test_hook(owner.selection);
        if successful_selection.is_some() {
            self.prefetch_screen_previews(Vec::new()).await;
        }
    }

    #[cfg(test)]
    pub(crate) fn install_scan_completion_wake_test_hook(
        &self,
        selection: SelectionToken,
        marker: std::sync::Arc<tokio::sync::Notify>,
    ) {
        *self.scan_completion_wake_test_hook.lock().unwrap() =
            Some(crate::service::ScanCompletionWakeTestHook { selection, marker });
    }

    #[cfg(test)]
    fn notify_scan_completion_wake_test_hook(&self, selection: SelectionToken) {
        let marker = {
            let mut hook = self.scan_completion_wake_test_hook.lock().unwrap();
            if hook
                .as_ref()
                .is_some_and(|hook| hook.selection == selection)
            {
                hook.take().map(|hook| hook.marker)
            } else {
                None
            }
        };
        if let Some(marker) = marker {
            marker.notify_one();
        }
    }
}

fn mark_unavailable_selection(
    state: &mut ServiceState,
    owner: ScanOwner,
) -> Result<(), AppServiceError> {
    let selection = state.libraries.catalog().load_app_state()?.active_selection;
    let is_canonical_root = selection
        .as_ref()
        .and_then(|selection| selection.relative_folder.to_path_buf().ok())
        .is_some_and(|path| path.as_os_str().is_empty());
    if is_canonical_root {
        state
            .libraries
            .catalog_mut()
            .mark_root_offline(owner.selection.library_id)?;
    } else {
        state
            .libraries
            .catalog_mut()
            .mark_group_offline(owner.selection.library_id, owner.selection.group_id)?;
    }
    Ok(())
}

fn progress_dto(progress: photo_indexer::ScanProgress) -> crate::ScanProgressDto {
    crate::ScanProgressDto {
        discovered: progress.discovered,
        shaped: progress.shaped,
        enriched: progress.enriched,
        total: progress.total,
    }
}
