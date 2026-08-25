//! Scan orchestration lives here so host adapters only consume update DTOs.

use std::path::Path;
use std::time::Duration;

use photo_catalog::CatalogError;
use photo_core::FolderPolicyEngine;
use photo_indexer::{IndexEvent, Indexer, ScanRequest};

use crate::{AppService, AppServiceError, BootstrapState, WallUpdate};

impl AppService {
    /// Starts a cancellable scan after persisting the active selection. The catalog lock is
    /// released before any source read or event wait.
    pub async fn start_scan(&self, folder: &Path) -> Result<BootstrapState, AppServiceError> {
        let bootstrap = self.open_recent(folder)?;
        let (library_id, selection_root, library_root, group_id, generation) = {
            let mut state = self.state()?;
            let stored = state.libraries.catalog().load_app_state()?;
            let selection = stored
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
            let generation = state
                .libraries
                .catalog_mut()
                .begin_generation(selection.library_id)?;
            (
                selection.library_id,
                root.clone().join(&selected),
                root,
                group,
                generation,
            )
        };
        let indexer = Indexer::with_scheduler(
            self.metadata_reader.clone(),
            FolderPolicyEngine::new(Vec::new()).map_err(|error| {
                AppServiceError::LibrarySetup(std::io::Error::other(error.to_string()))
            })?,
            self.scheduler.clone(),
        );
        let handle = match indexer.start(
            ScanRequest::new(selection_root.clone())
                .for_library(library_id)
                .roots(library_root, selection_root)
                .for_folder_group(group_id),
        ) {
            Ok(handle) => handle,
            Err(photo_indexer::IndexError::RootUnavailable(_)) => {
                let mut state = self.state()?;
                state
                    .libraries
                    .catalog_mut()
                    .mark_root_offline(library_id)?;
                let _ = self.updates.send(WallUpdate::SourceUnavailable {
                    source_id: library_id.as_uuid().hyphenated().to_string(),
                });
                return Ok(bootstrap);
            }
            Err(error) => {
                return Err(AppServiceError::LibrarySetup(std::io::Error::other(
                    error.to_string(),
                )));
            }
        };
        let cancel = handle.cancellation_sender();
        if let Ok(mut state) = self.state.lock() {
            state.active_cancel = Some(cancel);
            state.active_scan = Some((library_id, generation));
        }
        let service = self.clone();
        tokio::spawn(async move {
            service
                .drain_scan(handle, library_id, generation, group_id)
                .await;
        });
        Ok(bootstrap)
    }

    pub(crate) async fn reconcile_existing(&self) {
        let details = (|| -> Result<_, AppServiceError> {
            let state = self.state()?;
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
            Ok((
                selection.library_id,
                root.clone().join(selected),
                root,
                group,
            ))
        })();
        let Ok((library_id, selection_root, library_root, group_id)) = details else {
            return;
        };
        let generation = match self.state.lock() {
            Ok(mut state) => state.libraries.catalog_mut().begin_generation(library_id),
            Err(_) => return,
        };
        let Ok(generation) = generation else {
            return;
        };
        let indexer = match FolderPolicyEngine::new(Vec::new()) {
            Ok(policy) => Indexer::with_scheduler(
                self.metadata_reader.clone(),
                policy,
                self.scheduler.clone(),
            ),
            Err(_) => return,
        };
        let handle = match indexer.start(
            ScanRequest::new(selection_root.clone())
                .for_library(library_id)
                .roots(library_root, selection_root)
                .for_folder_group(group_id),
        ) {
            Ok(handle) => handle,
            Err(photo_indexer::IndexError::RootUnavailable(_)) => {
                if let Ok(mut state) = self.state.lock() {
                    let _ = state.libraries.catalog_mut().mark_root_offline(library_id);
                }
                let _ = self.updates.send(WallUpdate::SourceUnavailable {
                    source_id: library_id.as_uuid().hyphenated().to_string(),
                });
                return;
            }
            Err(_) => return,
        };
        if let Ok(mut state) = self.state.lock() {
            state.active_cancel = Some(handle.cancellation_sender());
            state.active_scan = Some((library_id, generation));
        }
        self.drain_scan(handle, library_id, generation, group_id)
            .await;
    }

    async fn drain_scan(
        &self,
        mut handle: photo_indexer::ScanHandle,
        library_id: photo_domain::LibraryId,
        generation: u64,
        group_id: photo_domain::FolderGroupId,
    ) {
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
            let result = if let Ok(mut state) = self.state.lock() {
                let mut writer = photo_indexer::CatalogWriter::new(
                    state.libraries.catalog_mut(),
                    library_id,
                    generation,
                );
                writer.apply_batch(&writer_events)
            } else {
                return;
            };
            if result.is_err() {
                return;
            }
            if !shaped.is_empty() {
                let assets = if let Ok(state) = self.state.lock() {
                    state
                        .libraries
                        .catalog()
                        .wall_page(group_id, photo_catalog::WallOrder::Provisional, None, 250)
                        .ok()
                        .map(|page| {
                            page.items
                                .into_iter()
                                .filter(|asset| shaped.contains(&asset.id))
                                .map(|asset| crate::WallAsset {
                                    id: asset.id.as_uuid().hyphenated().to_string(),
                                    captured_at_utc: asset.captured_at_utc,
                                    width: asset.width,
                                    height: asset.height,
                                    wall_thumbnail: None,
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    Vec::new()
                };
                if !assets.is_empty() {
                    let _ = self.updates.send(WallUpdate::CatalogBatch {
                        assets,
                        order_state: crate::OrderState::Provisional,
                        progress: progress_dto(last),
                    });
                }
            }
            if last.discovered > 0 {
                let _ = self.updates.send(WallUpdate::Progress {
                    progress: progress_dto(last),
                });
            }
            writer_events.clear();
        }
        let successful = handle
            .join()
            .await
            .map(|summary| !summary.cancelled)
            .unwrap_or(false);
        let recent = if let Ok(mut state) = self.state.lock() {
            if state.active_scan != Some((library_id, generation)) {
                return;
            }
            state.active_scan = None;
            state.active_cancel = None;
            if successful {
                if state
                    .libraries
                    .catalog_mut()
                    .complete_generation(library_id, generation)
                    .is_err()
                {
                    return;
                }
                Some(state.recent_derivative_ids.clone())
            } else {
                None
            }
        } else {
            return;
        };
        if let Some(recent) = recent {
            let _ = self.updates.send(WallUpdate::MetadataSettled {
                source_id: library_id.as_uuid().hyphenated().to_string(),
            });
            if !recent.is_empty() {
                self.prefetch_screen_previews(recent).await;
            }
        }
    }
}

fn progress_dto(progress: photo_indexer::ScanProgress) -> crate::ScanProgressDto {
    crate::ScanProgressDto {
        discovered: progress.discovered,
        shaped: progress.shaped,
        enriched: progress.enriched,
        total: progress.total,
    }
}
