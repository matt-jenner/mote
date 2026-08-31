use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use photo_domain::GalleryScope;
use tokio::sync::{Notify, broadcast, watch};

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
    generation: u64,
    cancel_requested: bool,
    sender: Option<watch::Sender<bool>>,
    reconciled_recovery_epoch: u64,
    pending_recovery_epoch: Option<u64>,
}

#[cfg(test)]
pub(crate) struct LifecyclePublishTestGate {
    pub(crate) state: ScanLifecycle,
    pub(crate) entered: std::sync::mpsc::Sender<()>,
    pub(crate) gate: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl ScanControl {
    fn new(state: ScanLifecycle) -> Self {
        Self {
            state,
            generation: 0,
            cancel_requested: false,
            sender: None,
            reconciled_recovery_epoch: 0,
            pending_recovery_epoch: None,
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
    pub(crate) terminal_event_id: AtomicU64,
    pub(crate) next_client_token: AtomicU64,
    pub(crate) settled: std::sync::atomic::AtomicBool,
    pub(crate) source_unavailable_reported: std::sync::atomic::AtomicBool,
    pub(crate) client_demand: Mutex<HashMap<u64, ClientDemand>>,
    pub(crate) lease_wake: Arc<Notify>,
    pub(crate) lease_reaper_started: std::sync::atomic::AtomicBool,
    #[cfg(test)]
    pub(crate) cancel_attempt_test_hook: Mutex<Option<std::sync::mpsc::Sender<()>>>,
    #[cfg(test)]
    pub(crate) lifecycle_publish_test_gate: Mutex<Option<LifecyclePublishTestGate>>,
}

#[derive(Clone)]
pub(crate) struct ClientDemand {
    pub(crate) client_id: String,
    pub(crate) origin: SubscriptionOrigin,
    pub(crate) scope: GalleryScope,
    pub(crate) interaction: InteractionState,
    pub(crate) lease_until: tokio::time::Instant,
    pub(crate) scope_sender: watch::Sender<GalleryScope>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SubscriptionOrigin {
    Hosted,
    Desktop,
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
            terminal_event_id: AtomicU64::new(0),
            next_client_token: AtomicU64::new(1),
            settled: std::sync::atomic::AtomicBool::new(settled),
            source_unavailable_reported: std::sync::atomic::AtomicBool::new(false),
            client_demand: Mutex::new(HashMap::new()),
            lease_wake: Arc::new(Notify::new()),
            lease_reaper_started: std::sync::atomic::AtomicBool::new(false),
            #[cfg(test)]
            cancel_attempt_test_hook: Mutex::new(None),
            #[cfg(test)]
            lifecycle_publish_test_gate: Mutex::new(None),
        })
    }

    pub(crate) fn lifecycle_receiver(&self) -> watch::Receiver<ScanLifecycle> {
        self.scan_lifecycle.subscribe()
    }

    #[cfg(test)]
    fn wait_lifecycle_publish_test_gate(&self, state: ScanLifecycle) {
        let gate = {
            let mut configured = self
                .lifecycle_publish_test_gate
                .lock()
                .expect("lifecycle publish test gate poisoned");
            if configured
                .as_ref()
                .is_some_and(|configured| configured.state == state)
            {
                configured.take()
            } else {
                None
            }
        };
        let Some(gate) = gate else {
            return;
        };
        gate.entered.send(()).unwrap();
        let (released, changed) = &*gate.gate;
        let mut released = released.lock().unwrap();
        while !*released {
            released = changed.wait(released).unwrap();
        }
    }

    #[cfg(test)]
    pub(crate) fn admit_scan(&self) -> Option<u64> {
        self.admit_scan_at_recovery_epoch(0)
    }

    pub(crate) fn admit_scan_at_recovery_epoch(&self, recovery_epoch: u64) -> Option<u64> {
        self.admit_scan_from(recovery_epoch, false)
    }

    pub(crate) fn admit_recovery_scan(&self, recovery_epoch: u64) -> Option<u64> {
        self.admit_scan_from(recovery_epoch, true)
    }

    fn admit_scan_from(&self, recovery_epoch: u64, settled_recovery: bool) -> Option<u64> {
        let mut control = self.scan_cancel.lock().expect("scan control poisoned");
        let admissible = if settled_recovery {
            matches!(
                control.state,
                ScanLifecycle::Completed
                    | ScanLifecycle::Cancelled
                    | ScanLifecycle::SourceUnavailable
                    | ScanLifecycle::Failed
            )
        } else {
            matches!(
                control.state,
                ScanLifecycle::Idle | ScanLifecycle::SourceUnavailable | ScanLifecycle::Failed
            )
        };
        if !admissible {
            return None;
        }
        control.generation = control.generation.wrapping_add(1).max(1);
        control.state = ScanLifecycle::Starting;
        control.cancel_requested = false;
        control.sender = None;
        control.pending_recovery_epoch = Some(recovery_epoch);
        self.terminal_event_id.store(0, Ordering::Release);
        #[cfg(test)]
        self.wait_lifecycle_publish_test_gate(ScanLifecycle::Starting);
        self.scan_lifecycle.send_replace(ScanLifecycle::Starting);
        Some(control.generation)
    }

    pub(crate) fn recovery_required(&self, recovery_epoch: u64) -> bool {
        let control = self.scan_cancel.lock().expect("scan control poisoned");
        control.pending_recovery_epoch.is_some()
            || control.reconciled_recovery_epoch < recovery_epoch
    }

    /// Installs the indexer's cancellation sender after scan admission. A
    /// cancellation can race this handoff, so the sender is cancelled before
    /// it is published when the control state has already been terminated.
    pub(crate) fn install_scan_sender(&self, generation: u64, sender: watch::Sender<bool>) -> bool {
        let cancel_now = {
            let mut control = self.scan_cancel.lock().expect("scan control poisoned");
            if control.generation == generation
                && control.state == ScanLifecycle::Starting
                && !control.cancel_requested
            {
                control.state = ScanLifecycle::Running;
                control.sender = Some(sender.clone());
                #[cfg(test)]
                self.wait_lifecycle_publish_test_gate(ScanLifecycle::Running);
                self.scan_lifecycle.send_replace(ScanLifecycle::Running);
                false
            } else {
                true
            }
        };
        if cancel_now {
            let _ = sender.send(true);
        }
        cancel_now
    }

    /// Requests cancellation without holding a guard across an await. The
    /// sender handoff and cancellation flag share one short synchronous lock,
    /// so a replacement cannot miss a sender installed concurrently.
    pub(crate) fn request_cancel(&self) {
        #[cfg(test)]
        if let Some(hook) = self
            .cancel_attempt_test_hook
            .lock()
            .expect("cancel test hook poisoned")
            .take()
        {
            let _ = hook.send(());
        }
        let sender = {
            let mut control = self.scan_cancel.lock().expect("scan control poisoned");
            if control.state.is_terminal() {
                return;
            }
            control.cancel_requested = true;
            control.state = ScanLifecycle::Cancelled;
            self.terminal_event_id.store(
                self.next_event_id.load(Ordering::Acquire),
                Ordering::Release,
            );
            #[cfg(test)]
            self.wait_lifecycle_publish_test_gate(ScanLifecycle::Cancelled);
            self.scan_lifecycle.send_replace(ScanLifecycle::Cancelled);
            control.sender.take()
        };
        if let Some(sender) = sender {
            let _ = sender.send(true);
        }
    }

    pub(crate) fn finish_scan(
        &self,
        generation: u64,
        requested_state: ScanLifecycle,
    ) -> ScanLifecycle {
        let mut control = self.scan_cancel.lock().expect("scan control poisoned");
        if control.generation != generation || control.state.is_terminal() {
            return control.state;
        }
        let state = if control.cancel_requested || control.state == ScanLifecycle::Cancelled {
            ScanLifecycle::Cancelled
        } else {
            requested_state
        };
        let publication = if state == ScanLifecycle::Failed {
            Some(
                self.publication
                    .lock()
                    .expect("runtime publication poisoned"),
            )
        } else {
            None
        };
        let failure_event_id = publication
            .as_ref()
            .map(|_| self.publish_terminal_warning_locked().id);
        control.state = state;
        control.cancel_requested = state == ScanLifecycle::Cancelled;
        control.sender = None;
        if state == ScanLifecycle::Completed
            && let Some(recovery_epoch) = control.pending_recovery_epoch.take()
        {
            control.reconciled_recovery_epoch =
                control.reconciled_recovery_epoch.max(recovery_epoch);
        }
        self.terminal_event_id.store(
            failure_event_id.unwrap_or_else(|| self.next_event_id.load(Ordering::Acquire)),
            Ordering::Release,
        );
        #[cfg(test)]
        self.wait_lifecycle_publish_test_gate(state);
        self.scan_lifecycle.send_replace(state);
        drop(publication);
        state
    }

    pub(crate) fn cancellation_requested(&self) -> bool {
        let control = self.scan_cancel.lock().expect("scan control poisoned");
        control.cancel_requested || control.state == ScanLifecycle::Cancelled
    }

    pub(crate) fn current_generation(&self) -> u64 {
        self.scan_cancel
            .lock()
            .expect("scan control poisoned")
            .generation
    }

    fn publish_locked(&self, update: WallUpdate) -> SequencedWallUpdate {
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

    fn publish_now(&self, update: WallUpdate) -> SequencedWallUpdate {
        let _publication = self
            .publication
            .lock()
            .expect("runtime publication poisoned");
        self.publish_locked(update)
    }

    pub(crate) async fn publish(&self, update: WallUpdate) -> SequencedWallUpdate {
        self.publish_now(update)
    }

    fn publish_terminal_warning_locked(&self) -> SequencedWallUpdate {
        self.publish_locked(WallUpdate::Warning {
            selection_id: self.selection.id().to_owned(),
            source_id: self.selection.library_id.as_uuid().hyphenated().to_string(),
            asset_id: None,
            warning: crate::WallWarningState {
                code: "catalogUnavailable".to_owned(),
                retryable: true,
            },
        })
    }

    pub(crate) fn register(
        &self,
        client_id: String,
        scope: GalleryScope,
        origin: SubscriptionOrigin,
    ) -> (u64, watch::Receiver<GalleryScope>) {
        let token = self.next_client_token.fetch_add(1, Ordering::AcqRel);
        let interaction = if origin == SubscriptionOrigin::Hosted {
            InteractionState::Active
        } else {
            InteractionState::Idle
        };
        let (scope_sender, scope_receiver) = watch::channel(scope);
        let mut demand = self.client_demand.lock().expect("client demand poisoned");
        demand.insert(
            token,
            ClientDemand {
                client_id,
                origin,
                scope,
                interaction,
                lease_until: tokio::time::Instant::now()
                    + if interaction == InteractionState::Active {
                        std::time::Duration::from_secs(30)
                    } else {
                        std::time::Duration::ZERO
                    },
                scope_sender,
            },
        );
        (token, scope_receiver)
    }

    pub(crate) fn remove(&self, client_id: &str, token: u64) -> bool {
        let mut demand = self.client_demand.lock().expect("client demand poisoned");
        let _ = client_id;
        let hosted = demand
            .remove(&token)
            .is_some_and(|value| value.origin == SubscriptionOrigin::Hosted);
        self.lease_wake.notify_waiters();
        hosted
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

    pub(crate) fn has_active_lease(&self) -> bool {
        let now = tokio::time::Instant::now();
        self.client_demand
            .lock()
            .expect("client demand poisoned")
            .values()
            .any(|value| {
                value.origin == SubscriptionOrigin::Hosted
                    && value.interaction == InteractionState::Active
                    && value.lease_until > now
            })
    }

    pub(crate) fn start_lease_reaper(self: &Arc<Self>, engine: GalleryEngine) {
        if self
            .lease_reaper_started
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            return;
        }
        let weak = Arc::downgrade(self);
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        handle.spawn(async move {
            engine.refresh_scheduler_interaction().await;
            loop {
                let Some(runtime) = weak.upgrade() else {
                    break;
                };
                let lease_wake = runtime.lease_wake.clone();
                let notified = lease_wake.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                let deadline = runtime.client_demand.lock().ok().and_then(|demand| {
                    demand
                        .values()
                        .filter(|value| {
                            value.origin == SubscriptionOrigin::Hosted
                                && value.interaction == InteractionState::Active
                        })
                        .map(|value| value.lease_until)
                        .min()
                });
                drop(runtime);
                if let Some(deadline) = deadline {
                    tokio::select! {
                        _ = tokio::time::sleep_until(deadline) => {
                            if let Some(runtime) = weak.upgrade() {
                                runtime.expire_leases();
                            }
                            engine.refresh_scheduler_interaction().await;
                        }
                        _ = &mut notified => {}
                    }
                } else {
                    notified.await;
                }
            }
        });
    }

    pub(crate) fn expire_leases(&self) {
        let now = tokio::time::Instant::now();
        if let Ok(mut demand) = self.client_demand.lock() {
            for value in demand.values_mut() {
                if value.origin == SubscriptionOrigin::Hosted
                    && value.interaction == InteractionState::Active
                    && value.lease_until <= now
                {
                    value.interaction = InteractionState::Idle;
                }
            }
        }
    }

    pub(crate) async fn begin_scan(
        self: &Arc<Self>,
        engine: GalleryEngine,
    ) -> Result<(), crate::AppServiceError> {
        let settled = self.settled.load(Ordering::Acquire);
        let recovery_epoch = engine.source_recovery_epoch(self.selection.library_id);
        let admitted_generation = if settled {
            if !engine.ensure_runtime_source(self).await? {
                return Ok(());
            }
            if !engine.runtime_recovery_required(self, recovery_epoch)? {
                return Ok(());
            }
            self.admit_recovery_scan(recovery_epoch)
        } else {
            self.admit_scan_at_recovery_epoch(recovery_epoch)
        };
        let Some(generation) = admitted_generation else {
            if !settled && !engine.ensure_runtime_source(self).await? {
                return Ok(());
            }
            return Ok(());
        };
        let source_available = match engine.ensure_runtime_source(self).await {
            Ok(source_available) => source_available,
            Err(error) => {
                self.finish_scan(generation, ScanLifecycle::Failed);
                return Err(error);
            }
        };
        if !source_available {
            return Ok(());
        }
        // Keep the derivative lifecycle attached to this selection runtime.
        // Desktop shares this coordinator with AppService; hosted selections
        // retain their own coordinator namespace.
        self.coordinator
            .ensure_selection(self.selection.token())
            .await;
        match engine.start_runtime_scan(self.clone(), generation).await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.finish_scan(generation, ScanLifecycle::Failed);
                Err(error)
            }
        }
    }
}

impl Drop for SelectionRuntime {
    fn drop(&mut self) {
        self.lease_wake.notify_waiters();
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
    pub(crate) terminal_event_id: Option<u64>,
    pub(crate) last_seen_event_id: u64,
}

impl SelectionEventSubscription {
    pub async fn recv(&mut self) -> Option<SequencedWallUpdate> {
        loop {
            if self.terminal.is_none() {
                let lifecycle = *self.lifecycle.borrow();
                if matches!(lifecycle, ScanLifecycle::Cancelled | ScanLifecycle::Failed) {
                    self.terminal = Some(lifecycle);
                    self.terminal_event_id =
                        Some(self.runtime.terminal_event_id.load(Ordering::Acquire));
                }
            }
            if self.terminal == Some(ScanLifecycle::Cancelled) {
                return None;
            }
            let next = if let Some(event) = self.backlog.pop_front() {
                Ok(event)
            } else if self.terminal == Some(ScanLifecycle::Failed) {
                let watermark = self.terminal_event_id.unwrap_or_default();
                if self.last_seen_event_id >= watermark {
                    return None;
                }
                self.receiver.recv().await
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
                                if matches!(state, ScanLifecycle::Cancelled | ScanLifecycle::Failed) {
                                    self.terminal = Some(state);
                                    self.terminal_event_id = Some(
                                        self.runtime
                                            .terminal_event_id
                                            .load(Ordering::Acquire),
                                    );
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
            self.last_seen_event_id = self.last_seen_event_id.max(event.id);
            if self
                .terminal
                .is_some_and(|terminal| terminal == ScanLifecycle::Cancelled)
                || *self.lifecycle.borrow() == ScanLifecycle::Cancelled
            {
                return None;
            }
            if self.terminal == Some(ScanLifecycle::Failed)
                && event.id > self.terminal_event_id.unwrap_or_default()
            {
                continue;
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
        let hosted = self.runtime.remove(&self.client_id, self.client_token);
        self.engine
            .remove_runtime_if_dead(self.runtime.selection.group_id, &self.runtime);
        if hosted && let Ok(handle) = tokio::runtime::Handle::try_current() {
            let engine = self.engine.clone();
            handle.spawn(async move {
                engine.refresh_scheduler_interaction().await;
            });
        }
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
        let (token, _scope) = runtime.register(
            "client".to_owned(),
            GalleryScope::IncludeSubfolders,
            SubscriptionOrigin::Hosted,
        );
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
        assert!(runtime.admit_scan().is_some());
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
        assert_eq!(
            admitted
                .iter()
                .filter(|admitted| admitted.is_some())
                .count(),
            1
        );
    }

    #[test]
    fn recovery_epoch_advances_only_after_successful_completion() {
        let initial = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        let initial_generation = initial
            .admit_scan_at_recovery_epoch(6)
            .expect("initial scan should capture the current recovery epoch");
        initial.finish_scan(initial_generation, ScanLifecycle::Completed);
        assert!(!initial.recovery_required(6));

        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            true,
        );
        assert!(runtime.recovery_required(7));
        let failed = runtime
            .admit_recovery_scan(7)
            .expect("recovery scan should be admitted");
        assert!(runtime.admit_recovery_scan(7).is_none());
        assert_eq!(
            runtime.finish_scan(failed, ScanLifecycle::Failed),
            ScanLifecycle::Failed
        );
        assert!(runtime.recovery_required(7));

        let retry = runtime
            .admit_recovery_scan(7)
            .expect("failed recovery should remain retryable");
        assert_eq!(
            runtime.finish_scan(retry, ScanLifecycle::Completed),
            ScanLifecycle::Completed
        );
        assert!(!runtime.recovery_required(7));
        assert!(runtime.recovery_required(8));

        let cancelled = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            true,
        );
        cancelled
            .admit_recovery_scan(9)
            .expect("cancelled recovery should first be admitted");
        cancelled.request_cancel();
        assert!(cancelled.recovery_required(9));
        let retry_after_cancel = cancelled
            .admit_recovery_scan(9)
            .expect("cancelled recovery should remain retryable for the pending epoch");
        cancelled.finish_scan(retry_after_cancel, ScanLifecycle::Completed);
        assert!(!cancelled.recovery_required(9));
    }

    #[test]
    fn stale_transition_writers_cannot_regress_a_newer_terminal_state() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        let generation = runtime.admit_scan().expect("scan should be admitted");
        let (sender, _receiver) = watch::channel(false);
        assert!(!runtime.install_scan_sender(generation, sender));

        runtime.request_cancel();

        let (stale_sender, _stale_receiver) = watch::channel(false);
        assert!(runtime.install_scan_sender(generation, stale_sender));
        assert_eq!(
            runtime.finish_scan(generation, ScanLifecycle::Completed),
            ScanLifecycle::Cancelled
        );
        assert_eq!(
            *runtime.lifecycle_receiver().borrow(),
            ScanLifecycle::Cancelled
        );
    }

    #[test]
    fn stale_generation_writers_cannot_regress_a_newer_failed_or_completed_state() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        let older_generation = runtime.admit_scan().expect("first scan should be admitted");
        assert_eq!(
            runtime.finish_scan(older_generation, ScanLifecycle::Failed),
            ScanLifecycle::Failed
        );
        let newer_generation = runtime.admit_scan().expect("retry should be admitted");
        let (sender, _receiver) = watch::channel(false);
        assert!(!runtime.install_scan_sender(newer_generation, sender));
        assert_eq!(
            runtime.finish_scan(newer_generation, ScanLifecycle::Completed),
            ScanLifecycle::Completed
        );

        let (stale_sender, _stale_receiver) = watch::channel(false);
        assert!(runtime.install_scan_sender(older_generation, stale_sender));
        assert_eq!(
            runtime.finish_scan(older_generation, ScanLifecycle::Failed),
            ScanLifecycle::Completed
        );
        assert_eq!(
            *runtime.lifecycle_receiver().borrow(),
            ScanLifecycle::Completed
        );
    }

    #[test]
    fn cancellation_waits_for_the_admitted_starting_publication() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        let (entered_send, entered_receive) = std::sync::mpsc::channel();
        *runtime.lifecycle_publish_test_gate.lock().unwrap() = Some(LifecyclePublishTestGate {
            state: ScanLifecycle::Starting,
            entered: entered_send,
            gate: gate.clone(),
        });
        let starting_runtime = runtime.clone();
        let starting = std::thread::spawn(move || starting_runtime.admit_scan());
        entered_receive
            .recv_timeout(Duration::from_secs(1))
            .expect("starting publication did not reach the interleaving gate");

        let (finished_send, finished_receive) = std::sync::mpsc::channel();
        let cancelling_runtime = runtime.clone();
        let cancelling = std::thread::spawn(move || {
            cancelling_runtime.request_cancel();
            finished_send.send(()).unwrap();
        });
        assert!(
            finished_receive
                .recv_timeout(Duration::from_millis(20))
                .is_err(),
            "cancellation must wait until Starting is published"
        );

        let (released, changed) = &*gate;
        *released.lock().unwrap() = true;
        changed.notify_all();
        assert!(starting.join().unwrap().is_some());
        finished_receive
            .recv_timeout(Duration::from_secs(1))
            .expect("cancellation did not finish after Starting publication");
        cancelling.join().unwrap();
        assert_eq!(
            *runtime.lifecycle_receiver().borrow(),
            ScanLifecycle::Cancelled
        );
    }

    #[test]
    fn cancellation_survives_scan_control_lock_contention() {
        let runtime = SelectionRuntime::new(
            selection(),
            Arc::new(photo_indexer::IndexScheduler::new(Default::default())),
            false,
        );
        assert!(runtime.admit_scan().is_some());
        let (attempted_send, attempted_receive) = std::sync::mpsc::channel();
        *runtime.cancel_attempt_test_hook.lock().unwrap() = Some(attempted_send);
        let guard = runtime.scan_cancel.lock().unwrap();
        let (finished_send, finished_receive) = std::sync::mpsc::channel();
        let worker = {
            let runtime = runtime.clone();
            std::thread::spawn(move || {
                runtime.request_cancel();
                finished_send.send(()).unwrap();
            })
        };
        attempted_receive
            .recv_timeout(Duration::from_secs(1))
            .expect("cancelling worker did not attempt the contested path");
        assert!(
            finished_receive
                .recv_timeout(Duration::from_millis(20))
                .is_err(),
            "cancellation must wait for the scan-control lock"
        );
        drop(guard);
        finished_receive
            .recv_timeout(Duration::from_secs(1))
            .expect("cancellation worker did not complete after lock release");
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
        let generation = runtime.admit_scan().expect("scan should be admitted");
        runtime.finish_scan(generation, ScanLifecycle::Completed);

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
