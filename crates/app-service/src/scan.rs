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
        let scope = self
            .state()
            .ok()
            .and_then(|state| state.libraries.catalog().load_app_state().ok())
            .map(|state| state.gallery_scope)
            .unwrap_or(GalleryScope::IncludeSubfolders);
        self.start_desktop_update_bridge(&selection, scope, selection_token);
        self.mark_desktop_scan_started(selection_token);
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
        let mut subscription = self.gallery.subscribe(
            selection,
            format!("desktop-{}", selection.id()),
            scope,
            None,
        );
        let updates = self.updates.clone();
        let service = self.clone();
        tokio::spawn(async move {
            while let Some(event) = subscription.recv().await {
                let settled = matches!(event.update, WallUpdate::MetadataSettled { .. });
                let terminal = matches!(
                    event.update,
                    WallUpdate::MetadataSettled { .. } | WallUpdate::SourceUnavailable { .. }
                );
                let _ = updates.send(event.update);
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
        });
    }

    fn mark_desktop_scan_started(&self, selection: SelectionToken) {
        if let Ok(mut state) = self.state.lock() {
            state.active_scan = Some(crate::service::ScanOwner {
                selection,
                generation: 0,
            });
        }
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
                let Some((finished, active)) = service.gallery.runtime_scan_state(&selection)
                else {
                    service.clear_desktop_scan(selection_token);
                    return;
                };
                if !active {
                    service.clear_desktop_scan(selection_token);
                    return;
                }
                finished.notified().await;
            }
        });
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
