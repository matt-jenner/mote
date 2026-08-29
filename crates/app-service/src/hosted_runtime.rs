use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use photo_domain::GalleryScope;
use tokio::sync::{broadcast, watch};

use crate::{GalleryEngine, GallerySelection, InteractionState, WallUpdate};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ScanLifecycle {
    Idle,
    Starting,
    Running,
    Completed,
    Cancelled,
    SourceUnavailable,
    Failed,
}

impl ScanLifecycle {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Cancelled | Self::SourceUnavailable | Self::Failed
        )
    }
}

struct ScanControl {
    state: ScanLifecycle,
    cancel_requested: bool,
    sender: Option<watch::Sender<bool>>,
}

impl ScanControl {
    fn new(state: ScanLifecycle) -> Self {
        Self {
            state,
            cancel_requested: false,
            sender: None,
        }
    }
}

pub(crate) struct SelectionRuntime {
    pub(crate) selection: GallerySelection,
    scan_cancel: Mutex<ScanControl>,
    pub(crate) scan_lifecycle: watch::Sender<ScanLifecycle>,
    pub(crate) coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
    pub(crate) updates: broadcast::Sender<SequencedWallUpdate>,
    pub(crate) publication: Mutex<()>,
    pub(crate) history: Mutex<VecDeque<SequencedWallUpdate>>,
    pub(crate) next_event_id: AtomicU64,
    pub(crate) next_client_token: AtomicU64,
    pub(crate) settled: std::sync::atomic::AtomicBool,
    pub(crate) source_unavailable_reported: std::sync::atomic::AtomicBool,
    pub(crate) client_demand: Mutex<HashMap<u64, ClientDemand>>,
}

#[derive(Clone)]
pub(crate) struct ClientDemand {
    pub(crate) client_id: String,
    pub(crate) scope: GalleryScope,
    pub(crate) interaction: InteractionState,
    pub(crate) lease_until: tokio::time::Instant,
    pub(crate) scope_sender: watch::Sender<GalleryScope>,
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
        settled: bool,
    ) -> Arc<Self> {
        Self::new_with_coordinator(
            selection.clone(),
            Arc::new(
                crate::derivative_coordinator::DerivativeCoordinator::with_selection(
                    scheduler,
                    selection.token(),
                ),
            ),
            settled,
        )
    }

    pub(crate) fn new_with_coordinator(
        selection: GallerySelection,
        coordinator: Arc<crate::derivative_coordinator::DerivativeCoordinator>,
        settled: bool,
    ) -> Arc<Self> {
        let (updates, _) = broadcast::channel(256);
        let initial_lifecycle = if settled {
            ScanLifecycle::Completed
        } else {
            ScanLifecycle::Idle
        };
        let (scan_lifecycle, _) = watch::channel(initial_lifecycle);
        Arc::new(Self {
            selection: selection.clone(),
            scan_cancel: Mutex::new(ScanControl::new(initial_lifecycle)),
            scan_lifecycle,
            coordinator,
            updates,
            publication: Mutex::new(()),
            history: Mutex::new(VecDeque::with_capacity(256)),
            next_event_id: AtomicU64::new(0),
            next_client_token: AtomicU64::new(1),
            settled: std::sync::atomic::AtomicBool::new(settled),
            source_unavailable_reported: std::sync::atomic::AtomicBool::new(false),
            client_demand: Mutex::new(HashMap::new()),
        })
    }

    pub(crate) fn lifecycle_receiver(&self) -> watch::Receiver<ScanLifecycle> {
        self.scan_lifecycle.subscribe()
    }

    pub(crate) fn admit_scan(&self) -> bool {
        let admitted = {
            let mut control = self.scan_cancel.lock().expect("scan control poisoned");
            if !matches!(
                control.state,
                ScanLifecycle::Idle | ScanLifecycle::SourceUnavailable | ScanLifecycle::Failed
            ) {
                false
            } else {
                control.state = ScanLifecycle::Starting;
                control.cancel_requested = false;
                control.sender = None;
                true
            }
        };
        if admitted {
            self.scan_lifecycle.send_replace(ScanLifecycle::Starting);
        }
        admitted
    }

    /// Installs the indexer's cancellation sender after scan admission. A
    /// cancellation can race this handoff, so the sender is cancelled before
    /// it is published when the control state has already been terminated.
    pub(crate) fn install_scan_sender(&self, sender: watch::Sender<bool>) -> bool {
        let cancel_now = {
            let mut control = self.scan_cancel.lock().expect("scan control poisoned");
            if control.state == ScanLifecycle::Starting && !control.cancel_requested {
                control.state = ScanLifecycle::Running;
                control.sender = Some(sender.clone());
                false
            } else {
                true
            }
        };
        if cancel_now {
            let _ = sender.send(true);
        } else {
            self.scan_lifecycle.send_replace(ScanLifecycle::Running);
        }
        cancel_now
    }

    /// Requests cancellation without holding a guard across an await. The
    /// sender handoff and cancellation flag share one short synchronous lock,
    /// so a replacement cannot miss a sender installed concurrently.
    pub(crate) fn request_cancel(&self) {
        let sender = {
            let mut control = self.scan_cancel.lock().expect("scan control poisoned");
            control.cancel_requested = true;
            control.state = ScanLifecycle::Cancelled;
            control.sender.take()
        };
        self.scan_lifecycle.send_replace(ScanLifecycle::Cancelled);
        if let Some(sender) = sender {
            let _ = sender.send(true);
        }
    }

    pub(crate) fn finish_scan(&self, requested_state: ScanLifecycle) -> ScanLifecycle {
        let state = {
            let mut control = self.scan_cancel.lock().expect("scan control poisoned");
            let state = if control.cancel_requested || control.state == ScanLifecycle::Cancelled {
                ScanLifecycle::Cancelled
            } else {
                requested_state
            };
            control.state = state;
            control.cancel_requested = state == ScanLifecycle::Cancelled;
            control.sender = None;
            state
        };
        self.scan_lifecycle.send_replace(state);
        state
    }

    pub(crate) fn cancellation_requested(&self) -> bool {
        let control = self.scan_cancel.lock().expect("scan control poisoned");
        control.cancel_requested || control.state == ScanLifecycle::Cancelled
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

    pub(crate) fn register(
        &self,
        client_id: String,
        scope: GalleryScope,
    ) -> (u64, watch::Receiver<GalleryScope>) {
        let token = self.next_client_token.fetch_add(1, Ordering::AcqRel);
        let (scope_sender, scope_receiver) = watch::channel(scope);
        let mut demand = self.client_demand.lock().expect("client demand poisoned");
        demand.insert(
            token,
            ClientDemand {
                client_id,
                scope,
                interaction: InteractionState::Active,
                lease_until: tokio::time::Instant::now() + std::time::Duration::from_secs(30),
                scope_sender,
            },
        );
        (token, scope_receiver)
    }

    pub(crate) fn remove(&self, client_id: &str, token: u64) {
        let mut demand = self.client_demand.lock().expect("client demand poisoned");
        let _ = client_id;
        demand.remove(&token);
    }

    pub(crate) fn aggregate_scope(&self) -> GalleryScope {
        let demand = self.client_demand.lock().expect("client demand poisoned");
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
        if !engine.ensure_runtime_source(self).await? {
            return Ok(());
        }
        // Keep the derivative lifecycle attached to this selection runtime.
        // Desktop shares this coordinator with AppService; hosted selections
        // retain their own coordinator namespace.
        self.coordinator
            .ensure_selection(self.selection.token())
            .await;
        if self.settled.load(Ordering::Acquire) {
            return Ok(());
        }
        if !self.admit_scan() {
            return Ok(());
        }
        match engine.start_runtime_scan(self.clone()).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.finish_scan(ScanLifecycle::Failed);
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
    pub(crate) scope: watch::Receiver<GalleryScope>,
    pub(crate) lifecycle: watch::Receiver<ScanLifecycle>,
    pub(crate) lagged: bool,
    pub(crate) resync_after: Option<u64>,
    pub(crate) resume_after: Option<u64>,
    pub(crate) engine: GalleryEngine,
    pub(crate) terminal: Option<ScanLifecycle>,
}

impl SelectionEventSubscription {
    pub async fn recv(&mut self) -> Option<SequencedWallUpdate> {
        loop {
            if *self.lifecycle.borrow() == ScanLifecycle::Cancelled {
                self.terminal = Some(ScanLifecycle::Cancelled);
                return None;
            }
            let next = if let Some(event) = self.backlog.pop_front() {
                Ok(event)
            } else if self.terminal == Some(ScanLifecycle::Cancelled) {
                return None;
            } else if self.lagged {
                self.lagged = false;
                let head = self.runtime.next_event_id.load(Ordering::Acquire);
                let watermark = head.max(self.resync_after.take().unwrap_or_default());
                self.resume_after = Some(watermark);
                return Some(SequencedWallUpdate {
                    id: watermark,
                    update: WallUpdate::ResyncRequired {
                        selection_id: self.runtime.selection.id().to_owned(),
                    },
                });
            } else {
                tokio::select! {
                    event = self.receiver.recv() => event,
                    changed = self.lifecycle.changed() => {
                        match changed {
                            Ok(()) => {
                                let state = *self.lifecycle.borrow();
                                if state == ScanLifecycle::Cancelled {
                                    self.terminal = Some(state);
                                    return None;
                                }
                                continue;
                            }
                            Err(_) => return None,
                        }
                    }
                }
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
                .terminal
                .is_some_and(|terminal| terminal == ScanLifecycle::Cancelled)
                || *self.lifecycle.borrow() == ScanLifecycle::Cancelled
            {
                return None;
            }
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
        let scope = *self.scope.borrow();
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
                    .wall_records_for_assets_scoped(self.runtime.selection.group_id(), scope, &ids)
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
                    .wall_records_for_assets_scoped(self.runtime.selection.group_id(), scope, &ids)
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
                        scope,
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
        self.engine
            .remove_runtime_if_dead(self.runtime.selection.group_id, &self.runtime);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_domain::{FolderGroupId, LibraryId, RelativePathKey};
    use std::path::Path;
    use std::time::Duration;

    fn selection() -> GallerySelection {
        GallerySelection {
            id: "selection-00000000-0000-0000-0000-000000000001".to_owned(),
            library_id: LibraryId::new(),
            group_id: FolderGroupId::new(),
            relative_folder: RelativePathKey::from_relative_path(Path::new(".")).unwrap(),
            epoch: 0,
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
    async fn concurrent_publication_keeps_history_and_broadcast_order_identical() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
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

    #[tokio::test]
    async fn expired_interaction_lease_keeps_connected_scope_demand() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        let (token, _scope) =
            runtime.register("client".to_owned(), GalleryScope::IncludeSubfolders);
        runtime
            .client_demand
            .lock()
            .unwrap()
            .get_mut(&token)
            .unwrap()
            .lease_until = tokio::time::Instant::now() - std::time::Duration::from_secs(1);

        assert_eq!(runtime.aggregate_scope(), GalleryScope::IncludeSubfolders);
        runtime.remove("client", token);
        assert_eq!(runtime.aggregate_scope(), GalleryScope::CurrentFolder);
    }

    #[tokio::test]
    async fn scan_cancellation_wakes_multiple_lifecycle_waiters() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        let mut first = runtime.lifecycle_receiver();
        let mut second = runtime.lifecycle_receiver();
        assert!(runtime.admit_scan());
        runtime.request_cancel();

        tokio::time::timeout(Duration::from_secs(1), first.changed())
            .await
            .expect("first lifecycle waiter did not wake")
            .expect("first lifecycle sender closed");
        tokio::time::timeout(Duration::from_secs(1), second.changed())
            .await
            .expect("second lifecycle waiter did not wake")
            .expect("second lifecycle sender closed");
        assert_eq!(*first.borrow(), ScanLifecycle::Cancelled);
        assert_eq!(*second.borrow(), ScanLifecycle::Cancelled);
    }

    #[test]
    fn scan_admission_is_exactly_once_under_concurrent_callers() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        let barrier = Arc::new(std::sync::Barrier::new(3));
        let first_runtime = runtime.clone();
        let first_barrier = barrier.clone();
        let first = std::thread::spawn(move || {
            first_barrier.wait();
            first_runtime.admit_scan()
        });
        let second_runtime = runtime.clone();
        let second_barrier = barrier.clone();
        let second = std::thread::spawn(move || {
            second_barrier.wait();
            second_runtime.admit_scan()
        });
        barrier.wait();
        let admitted = [first.join().unwrap(), second.join().unwrap()];
        assert_eq!(admitted.iter().filter(|admitted| **admitted).count(), 1);
    }

    #[test]
    fn cancellation_survives_scan_control_lock_contention() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        assert!(runtime.admit_scan());
        let guard = runtime.scan_cancel.lock().unwrap();
        let worker = {
            let runtime = runtime.clone();
            std::thread::spawn(move || runtime.request_cancel())
        };
        drop(guard);
        worker.join().unwrap();
        assert_eq!(
            *runtime.lifecycle_receiver().borrow(),
            ScanLifecycle::Cancelled
        );
    }

    #[tokio::test]
    async fn scan_completion_wakes_multiple_lifecycle_waiters() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        let mut first = runtime.lifecycle_receiver();
        let mut second = runtime.lifecycle_receiver();
        assert!(runtime.admit_scan());
        runtime.finish_scan(ScanLifecycle::Completed);

        tokio::time::timeout(Duration::from_secs(1), first.changed())
            .await
            .expect("first lifecycle waiter did not wake")
            .expect("first lifecycle sender closed");
        tokio::time::timeout(Duration::from_secs(1), second.changed())
            .await
            .expect("second lifecycle waiter did not wake")
            .expect("second lifecycle sender closed");
        assert_eq!(*first.borrow(), ScanLifecycle::Completed);
        assert_eq!(*second.borrow(), ScanLifecycle::Completed);
    }
}
