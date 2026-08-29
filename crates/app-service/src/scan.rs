//! Desktop scan compatibility wrappers over the selection-explicit gallery engine.

use std::path::Path;

use crate::service::SelectionToken;
use crate::{AppService, AppServiceError, BootstrapState, GalleryScope, WallUpdate};

impl AppService {
    /// Starts a scan through the same selection runtime used by hosted clients.
    /// The legacy synchronous selection/bootstrap API remains unchanged for
    /// desktop callers.
    pub async fn start_scan(&self, folder: &Path) -> Result<BootstrapState, AppServiceError> {
        let (bootstrap, selection) = self.select_recent(folder)?;
        self.start_selected_scan(selection).await?;
        Ok(bootstrap)
    }

    pub(crate) async fn start_selected_scan(
        &self,
        selection_token: SelectionToken,
    ) -> Result<(), AppServiceError> {
        let selection = self.gallery.selection_from_token(selection_token)?;
        if !self.admit_desktop_scan(selection_token)? {
            return Ok(());
        }
        {
            // Keep bridge installation in the same transition critical section
            // as selection changes. A newer selection therefore cannot become
            // active between this check and the bridge subscription.
            let _transition = self
                .selection_transition
                .lock()
                .map_err(|_| AppServiceError::StatePoisoned)?;
            if !self.desktop_selection_is_current_locked(selection_token) {
                self.clear_desktop_scan(selection_token);
                return Ok(());
            }
            let scope = self
                .state()
                .ok()
                .and_then(|state| state.libraries.catalog().load_app_state().ok())
                .map(|state| state.gallery_scope)
                .unwrap_or(GalleryScope::IncludeSubfolders);
            self.start_desktop_update_bridge(&selection, scope, selection_token);
        }
        self.wake_derivative_workers();
        let result = self.gallery.ensure_running(&selection).await;
        if result.is_err() {
            self.clear_desktop_scan(selection_token);
        }
        self.spawn_desktop_scan_cleanup(selection_token, selection);
        result
    }

    pub(crate) async fn reconcile_existing(&self) {
        let Ok(selection_token) = self.active_selection_token() else {
            return;
        };
        let _ = self.start_selected_scan(selection_token).await;
    }

    fn start_desktop_update_bridge(
        &self,
        selection: &crate::GallerySelection,
        scope: GalleryScope,
        selection_token: SelectionToken,
    ) {
        let after_event_id = self.gallery.current_event_id(selection);
        let mut subscription = self.gallery.subscribe(
            selection,
            format!("desktop-{}", selection.id()),
            scope,
            Some(after_event_id),
        );
        let bridge_id = self
            .next_bridge_id
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel)
            .wrapping_add(1)
            .max(1);
        let bridges = self.desktop_bridges.clone();
        let service = self.clone();
        let task = tokio::spawn(async move {
            tokio::task::yield_now().await;
            while let Some(event) = subscription.recv().await {
                let update = event.update;
                let settled = matches!(&update, WallUpdate::MetadataSettled { .. });
                let terminal = matches!(
                    &update,
                    WallUpdate::MetadataSettled { .. } | WallUpdate::SourceUnavailable { .. }
                );
                if !service.forward_desktop_update(selection_token, bridge_id, update) {
                    break;
                }
                if terminal {
                    service.clear_desktop_scan(selection_token);
                    service.coordinator.wake();
                }
                if settled {
                    service.prefetch_screen_previews(Vec::new()).await;
                    #[cfg(test)]
                    service.notify_scan_completion_wake_test_hook(selection_token);
                }
                if terminal {
                    break;
                }
            }
            let owns_bridge = bridges
                .lock()
                .ok()
                .and_then(|mut bridges| {
                    bridges
                        .get(&selection_token)
                        .is_some_and(|entry| entry.id == bridge_id)
                        .then(|| bridges.remove(&selection_token).is_some())
                })
                .unwrap_or(false);
            if owns_bridge {
                service.clear_desktop_scan(selection_token);
                service.coordinator.wake();
            }
        });
        let abort = task.abort_handle();
        let previous = {
            let mut bridges = self
                .desktop_bridges
                .lock()
                .expect("desktop bridge registry poisoned");
            bridges.insert(
                selection_token,
                crate::service::DesktopBridgeEntry {
                    id: bridge_id,
                    abort,
                },
            )
        };
        if let Some(previous) = previous {
            previous.abort.abort();
        }
        if task.is_finished()
            && let Ok(mut bridges) = self.desktop_bridges.lock()
            && bridges
                .get(&selection_token)
                .is_some_and(|entry| entry.id == bridge_id)
        {
            bridges.remove(&selection_token);
        }
    }

    fn admit_desktop_scan(&self, selection: SelectionToken) -> Result<bool, AppServiceError> {
        let _transition = self
            .selection_transition
            .lock()
            .map_err(|_| AppServiceError::StatePoisoned)?;
        let mut state = self.state()?;
        let stored = state.libraries.catalog().load_app_state()?;
        let Some(active) = stored.active_selection else {
            return Ok(false);
        };
        let group = state
            .libraries
            .catalog()
            .folder_group_for_path(active.library_id, &active.relative_folder)?;
        if active.library_id != selection.library_id
            || group != Some(selection.group_id)
            || state.selection_epoch != selection.epoch
            || state.protected_group != Some(selection.group_id)
        {
            return Ok(false);
        }
        if state
            .active_scan
            .is_some_and(|owner| owner.selection == selection)
        {
            return Ok(false);
        }
        state.active_scan = Some(crate::service::ScanOwner {
            selection,
            generation: 0,
        });
        Ok(true)
    }

    fn desktop_selection_is_current_locked(&self, selection: SelectionToken) -> bool {
        let Ok(state) = self.state() else {
            return false;
        };
        let Ok(Some(active)) = state
            .libraries
            .catalog()
            .load_app_state()
            .map(|state| state.active_selection)
        else {
            return false;
        };
        let Ok(group) = state
            .libraries
            .catalog()
            .folder_group_for_path(active.library_id, &active.relative_folder)
        else {
            return false;
        };
        active.library_id == selection.library_id
            && group == Some(selection.group_id)
            && state.selection_epoch == selection.epoch
            && state.protected_group == Some(selection.group_id)
    }

    fn forward_desktop_update(
        &self,
        selection: SelectionToken,
        bridge_id: u64,
        update: WallUpdate,
    ) -> bool {
        let Ok(_transition) = self.selection_transition.lock() else {
            return false;
        };
        let owns_bridge = self.desktop_bridges.lock().ok().is_some_and(|bridges| {
            bridges
                .get(&selection)
                .is_some_and(|bridge| bridge.id == bridge_id)
        });
        if !owns_bridge {
            return false;
        }
        if !self.desktop_selection_is_current_locked(selection) {
            return false;
        }
        let _ = self.updates.send(update);
        true
    }

    fn clear_desktop_scan(&self, selection: SelectionToken) {
        if let Ok(mut state) = self.state.lock()
            && state
                .active_scan
                .is_some_and(|owner| owner.selection == selection)
        {
            state.active_scan = None;
        }
    }

    fn spawn_desktop_scan_cleanup(
        &self,
        selection_token: SelectionToken,
        selection: crate::GallerySelection,
    ) {
        let service = self.clone();
        tokio::spawn(async move {
            loop {
                let Some(mut lifecycle) = service.gallery.runtime_scan_state(&selection) else {
                    service.clear_desktop_scan(selection_token);
                    return;
                };
                if lifecycle.borrow().is_terminal() {
                    service.clear_desktop_scan(selection_token);
                    return;
                }
                if lifecycle.changed().await.is_err() {
                    service.clear_desktop_scan(selection_token);
                    return;
                }
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn desktop_bridge_count_for_test(&self) -> usize {
        self.desktop_bridges
            .lock()
            .expect("desktop bridge registry poisoned")
            .len()
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

pub(crate) fn progress_dto(progress: photo_indexer::ScanProgress) -> crate::ScanProgressDto {
    crate::ScanProgressDto {
        discovered: progress.discovered,
        shaped: progress.shaped,
        enriched: progress.enriched,
        total: progress.total,
    }
}
