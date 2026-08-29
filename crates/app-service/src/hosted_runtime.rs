use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use photo_domain::GalleryScope;
use tokio::sync::{broadcast, watch};

use crate::{GalleryEngine, GallerySelection, InteractionState, WallUpdate};

pub(crate) struct SelectionRuntime {
    pub(crate) selection: GallerySelection,
    pub(crate) scan_cancel: tokio::sync::Mutex<Option<watch::Sender<bool>>>,
    #[allow(dead_code)]
    pub(crate) coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
    pub(crate) updates: broadcast::Sender<SequencedWallUpdate>,
    pub(crate) publication: Mutex<()>,
    pub(crate) history: Mutex<VecDeque<SequencedWallUpdate>>,
    pub(crate) next_event_id: AtomicU64,
    pub(crate) next_client_token: AtomicU64,
    pub(crate) settled: std::sync::atomic::AtomicBool,
    pub(crate) client_demand: Mutex<HashMap<u64, ClientDemand>>,
}

#[derive(Clone)]
pub(crate) struct ClientDemand {
    pub(crate) client_id: String,
    pub(crate) scope: GalleryScope,
    pub(crate) interaction: InteractionState,
    pub(crate) lease_until: tokio::time::Instant,
}

#[derive(Clone, Debug)]
pub struct SequencedWallUpdate {
    pub id: u64,
    pub update: WallUpdate,
}

impl SelectionRuntime {
    pub(crate) fn new(
        selection: GallerySelection,
        scheduler: Arc<photo_indexer::IndexScheduler>,
    ) -> Arc<Self> {
        let (updates, _) = broadcast::channel(256);
        Arc::new(Self {
            selection: selection.clone(),
            scan_cancel: tokio::sync::Mutex::new(None),
            coordinator: Arc::new(
                crate::derivative_coordinator::DerivativeCoordinator::with_selection(
                    scheduler,
                    selection.token(),
                ),
            ),
            updates,
            publication: Mutex::new(()),
            history: Mutex::new(VecDeque::with_capacity(256)),
            next_event_id: AtomicU64::new(0),
            next_client_token: AtomicU64::new(1),
            settled: std::sync::atomic::AtomicBool::new(false),
            client_demand: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) async fn publish(&self, update: WallUpdate) -> SequencedWallUpdate {
        let _publication = self
            .publication
            .lock()
            .expect("runtime publication poisoned");
        let event = SequencedWallUpdate {
            id: self.next_event_id.fetch_add(1, Ordering::AcqRel) + 1,
            update,
        };
        let mut history = self.history.lock().expect("runtime history poisoned");
        if history.len() == 256 {
            history.pop_front();
        }
        history.push_back(event.clone());
        let _ = self.updates.send(event.clone());
        drop(history);
        event
    }

    pub(crate) fn register(&self, client_id: String, scope: GalleryScope) -> u64 {
        let token = self.next_client_token.fetch_add(1, Ordering::AcqRel);
        let mut demand = self.client_demand.lock().expect("client demand poisoned");
        demand.insert(
            token,
            ClientDemand {
                client_id,
                scope,
                interaction: InteractionState::Active,
                lease_until: tokio::time::Instant::now() + std::time::Duration::from_secs(30),
            },
        );
        token
    }

    pub(crate) fn remove(&self, client_id: &str, token: u64) {
        let mut demand = self.client_demand.lock().expect("client demand poisoned");
        let _ = client_id;
        demand.remove(&token);
    }

    /// Cancel an idle scan as soon as the last live subscription goes away.
    ///
    /// The scan task owns the runtime while it drains its indexer handle, so
    /// this is deliberately best-effort and synchronous: the sender is the
    /// actual handle-owned cancellation channel and the task will finish its
    /// normal cleanup path after observing it.
    pub(crate) fn cancel_if_idle(&self) {
        let demand_empty = self.client_demand.lock().is_ok_and(|mut demand| {
            let now = tokio::time::Instant::now();
            demand.retain(|_, value| value.lease_until > now);
            demand.is_empty()
        });
        if !demand_empty {
            return;
        }
        if let Ok(cancel) = self.scan_cancel.try_lock()
            && let Some(sender) = cancel.as_ref()
        {
            let _ = sender.send(true);
        }
    }

    pub(crate) fn aggregate_scope(&self) -> GalleryScope {
        let mut demand = self.client_demand.lock().expect("client demand poisoned");
        let now = tokio::time::Instant::now();
        demand.retain(|_, value| value.lease_until > now);
        if demand
            .values()
            .any(|value| value.scope == GalleryScope::IncludeSubfolders)
        {
            GalleryScope::IncludeSubfolders
        } else {
            GalleryScope::CurrentFolder
        }
    }

    pub(crate) async fn begin_scan(
        self: &Arc<Self>,
        engine: GalleryEngine,
    ) -> Result<(), crate::AppServiceError> {
        if self.settled.load(Ordering::Acquire) {
            return Ok(());
        }
        let (placeholder, _) = watch::channel(false);
        {
            let mut cancel = self.scan_cancel.lock().await;
            if cancel.is_some() {
                return Ok(());
            }
            *cancel = Some(placeholder);
        }
        match engine.start_runtime_scan(self.clone()).await {
            Ok(()) => Ok(()),
            Err(error) => {
                *self.scan_cancel.lock().await = None;
                Err(error)
            }
        }
    }
}

pub struct SelectionEventSubscription {
    pub(crate) backlog: VecDeque<SequencedWallUpdate>,
    pub(crate) receiver: broadcast::Receiver<SequencedWallUpdate>,
    pub(crate) runtime: Arc<SelectionRuntime>,
    pub(crate) client_id: String,
    pub(crate) client_token: u64,
    pub(crate) scope: GalleryScope,
    pub(crate) lagged: bool,
    pub(crate) resume_after: Option<u64>,
    pub(crate) engine: GalleryEngine,
}

impl SelectionEventSubscription {
    pub async fn recv(&mut self) -> Option<SequencedWallUpdate> {
        loop {
            let next = if let Some(event) = self.backlog.pop_front() {
                Ok(event)
            } else if self.lagged {
                self.lagged = false;
                self.resume_after = Some(self.runtime.next_event_id.load(Ordering::Acquire));
                return Some(SequencedWallUpdate {
                    id: self.resume_after.unwrap_or_default(),
                    update: WallUpdate::ResyncRequired {
                        selection_id: self.runtime.selection.id().to_owned(),
                    },
                });
            } else {
                self.receiver.recv().await
            };
            let event = match next {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    self.lagged = true;
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            };
            if self
                .resume_after
                .is_some_and(|watermark| event.id <= watermark)
            {
                continue;
            }
            self.resume_after = None;
            if let Some(event) = self.filter(event) {
                return Some(event);
            }
        }
    }

    fn filter(&self, mut event: SequencedWallUpdate) -> Option<SequencedWallUpdate> {
        let state = self.engine.state.lock().ok()?;
        let catalog = state.libraries.catalog();
        match &mut event.update {
            WallUpdate::CatalogBatch { assets, .. } => {
                let ids = assets
                    .iter()
                    .filter_map(|a| {
                        uuid::Uuid::parse_str(&a.id)
                            .ok()
                            .map(photo_domain::AssetId::from_uuid)
                    })
                    .collect::<Vec<_>>();
                let allowed = catalog
                    .wall_records_for_assets_scoped(
                        self.runtime.selection.group_id(),
                        self.scope,
                        &ids,
                    )
                    .ok()?;
                assets.retain(|a| {
                    allowed
                        .iter()
                        .any(|record| record.id.as_uuid().hyphenated().to_string() == a.id)
                });
                if assets.is_empty() { None } else { Some(event) }
            }
            WallUpdate::DerivativesReady { derivatives, .. } => {
                let ids = derivatives
                    .iter()
                    .filter_map(|a| {
                        uuid::Uuid::parse_str(&a.asset_id)
                            .ok()
                            .map(photo_domain::AssetId::from_uuid)
                    })
                    .collect::<Vec<_>>();
                let allowed = catalog
                    .wall_records_for_assets_scoped(
                        self.runtime.selection.group_id(),
                        self.scope,
                        &ids,
                    )
                    .ok()?;
                derivatives.retain(|a| {
                    allowed
                        .iter()
                        .any(|record| record.id.as_uuid().hyphenated().to_string() == a.asset_id)
                });
                if derivatives.is_empty() {
                    None
                } else {
                    Some(event)
                }
            }
            WallUpdate::Warning {
                asset_id: Some(id), ..
            }
            | WallUpdate::WarningCleared {
                asset_id: Some(id), ..
            } => {
                let parsed = uuid::Uuid::parse_str(id)
                    .ok()
                    .map(photo_domain::AssetId::from_uuid)?;
                if catalog
                    .wall_records_for_assets_scoped(
                        self.runtime.selection.group_id(),
                        self.scope,
                        &[parsed],
                    )
                    .ok()?
                    .is_empty()
                {
                    None
                } else {
                    Some(event)
                }
            }
            _ => Some(event),
        }
    }
}

impl Drop for SelectionEventSubscription {
    fn drop(&mut self) {
        self.runtime.remove(&self.client_id, self.client_token);
        self.runtime.cancel_if_idle();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{FolderGroupId, LibraryId, RelativePathKey};
    use std::path::Path;

    fn selection() -> GallerySelection {
        GallerySelection {
            id: "selection-00000000-0000-0000-0000-000000000001".to_owned(),
            library_id: LibraryId::new(),
            group_id: FolderGroupId::new(),
            relative_folder: RelativePathKey::from_relative_path(Path::new(".")).unwrap(),
        }
    }

    fn progress(selection_id: &str, generation: u64) -> WallUpdate {
        WallUpdate::Progress {
            selection_id: selection_id.to_owned(),
            generation,
            progress: Default::default(),
        }
    }

    #[tokio::test]
    async fn final_demand_drop_cancels_the_actual_scan_sender() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
        );
        let (sender, mut receiver) = watch::channel(false);
        *runtime.scan_cancel.lock().await = Some(sender);
        let token = runtime.register("client".to_owned(), GalleryScope::CurrentFolder);

        runtime.remove("client", token);
        runtime.cancel_if_idle();

        assert!(receiver.changed().await.is_ok());
        assert!(*receiver.borrow());
    }

    #[tokio::test]
    async fn concurrent_publication_keeps_history_and_broadcast_order_identical() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
        );
        let mut receiver = runtime.updates.subscribe();
        let mut tasks = Vec::new();
        for generation in 0..32 {
            let runtime = runtime.clone();
            tasks.push(tokio::spawn(async move {
                runtime
                    .publish(progress(runtime.selection.id(), generation))
                    .await
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }

        let mut broadcast_ids = Vec::new();
        for _ in 0..32 {
            broadcast_ids.push(receiver.recv().await.unwrap().id);
        }
        let history_ids = runtime
            .history
            .lock()
            .unwrap()
            .iter()
            .map(|event| event.id)
            .collect::<Vec<_>>();
        assert_eq!(broadcast_ids, history_ids);
        assert_eq!(broadcast_ids, (1..=32).collect::<Vec<_>>());
    }
}
