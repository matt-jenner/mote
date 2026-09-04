//! Derivative request orchestration lives here so source paths never cross the DTO boundary.

use std::collections::HashSet;
use std::path::PathBuf;
#[cfg(debug_assertions)]
use std::sync::Arc;
#[cfg(debug_assertions)]
use std::sync::atomic::AtomicUsize;
#[cfg(any(test, debug_assertions))]
use std::sync::atomic::Ordering;

use photo_cache::{
    DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget, ImageDerivativeError,
    ImageDerivativeGenerator,
};
use photo_catalog::{Catalog, NewDerivative, TerminalDerivativeFailure, WallOrder};
use photo_domain::{
    AssetId, Availability, DerivativeId, FolderGroupId, GalleryScope, MediaKind, RelativePathKey,
};
use photo_indexer::{InteractionMode, JobPriority};

use crate::derivative_coordinator::{
    CollectionProgressToken, CommitPermit, RECENT_CAPACITY, WorkKey, WorkTicket,
};
use crate::service::{
    CollectionDriverAdmission, CollectionDriverOwnerId, CollectionDriverTake, SelectionToken,
};
use crate::{
    AppService, AppServiceError, DerivativeClass, DerivativePriority, DerivativeReference,
    DerivativeRequest, WallUpdate,
};

const DECODER_VERSION: &str = "image-0.25-v1";
const ASSET_DERIVATIVE_WARNING: &str = "derivative_generation_failed";
const TERMINAL_ASSET_DERIVATIVE_WARNING: &str = "derivative_generation_terminal";
const WALL_CACHE_DERIVATIVE_WARNING: &str = "wall_thumbnail_cache_unavailable";
const SCREEN_CACHE_DERIVATIVE_WARNING: &str = "screen_preview_cache_unavailable";
const MAX_DERIVATIVE_WORKERS: usize = 2;

fn record_timing_stage(stage: &'static str, started: std::time::Instant) {
    tracing::debug!(
        target: "photo_viewer::timing",
        stage,
        elapsed_nanos = started.elapsed().as_nanos() as u64,
        "derivative_stage"
    );
}

#[derive(Debug)]
enum DerivativeWorkError {
    Image(ImageDerivativeError),
    WorkerUnavailable,
    PreviewSuperseded,
    WallThumbnailRequired,
}

struct AdmittedCommit {
    completion: tokio::sync::oneshot::Receiver<()>,
}

struct GenerationFailure {
    error: DerivativeWorkError,
    permit: Option<CommitPermit>,
}

enum PreviewPrefetchOutcome {
    Complete,
    Blocked,
    Stale,
    Terminate,
}

impl From<ImageDerivativeError> for DerivativeWorkError {
    fn from(error: ImageDerivativeError) -> Self {
        Self::Image(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DerivativeFailureScope {
    Asset,
    CacheWide,
}

impl DerivativeWorkError {
    fn scope(&self) -> DerivativeFailureScope {
        match self {
            Self::Image(ImageDerivativeError::Io(_))
            | Self::Image(ImageDerivativeError::Decode(_)) => DerivativeFailureScope::Asset,
            Self::Image(ImageDerivativeError::UnsupportedTarget)
            | Self::Image(ImageDerivativeError::BudgetAuthorizationRequired)
            | Self::Image(ImageDerivativeError::InvalidSpecification) => {
                DerivativeFailureScope::CacheWide
            }
            Self::Image(ImageDerivativeError::Cache(_))
            | Self::Image(ImageDerivativeError::Catalog(_))
            | Self::WorkerUnavailable
            | Self::PreviewSuperseded
            | Self::WallThumbnailRequired => DerivativeFailureScope::CacheWide,
        }
    }
}

#[derive(Clone)]
pub(crate) struct PendingDerivative {
    id: AssetId,
    source: PathBuf,
    cached_source: Option<PathBuf>,
    availability: Availability,
    spec: DerivativeSpec,
    group: FolderGroupId,
    class: DerivativeClass,
    selection: SelectionToken,
    completion_generation: u64,
}

struct CollectionDriverLifecycleOwner {
    service: AppService,
    admission: CollectionDriverAdmission,
    intent_consumed: bool,
    disarmed: bool,
    #[cfg(any(test, debug_assertions))]
    _task_guard: crate::service::DerivativeTaskGuard,
}

impl CollectionDriverLifecycleOwner {
    #[cfg(any(test, debug_assertions))]
    fn new_with_task_guard(
        service: AppService,
        admission: CollectionDriverAdmission,
        task_guard: crate::service::DerivativeTaskGuard,
    ) -> Self {
        Self {
            _task_guard: task_guard,
            service,
            admission,
            intent_consumed: false,
            disarmed: false,
        }
    }

    #[cfg(any(test, debug_assertions))]
    fn new(service: AppService, admission: CollectionDriverAdmission) -> Self {
        let task_guard = service.collection_driver_tracker.start();
        Self::new_with_task_guard(service, admission, task_guard)
    }

    #[cfg(not(any(test, debug_assertions)))]
    fn new(service: AppService, admission: CollectionDriverAdmission) -> Self {
        Self {
            service,
            admission,
            intent_consumed: false,
            disarmed: false,
        }
    }

    fn owner_id(&self) -> CollectionDriverOwnerId {
        self.admission.owner_id
    }

    fn take_pending_or_release(&mut self) -> CollectionDriverTake {
        let transition = self
            .service
            .collection_driver
            .take_pending_or_release(self.owner_id());
        match transition {
            CollectionDriverTake::Pending(_) => self.intent_consumed = true,
            CollectionDriverTake::Released | CollectionDriverTake::Stale => {
                self.disarmed = true;
            }
        }
        transition
    }
}

impl Drop for CollectionDriverLifecycleOwner {
    fn drop(&mut self) {
        if self.disarmed {
            return;
        }
        if let Some(successor) = self
            .service
            .collection_driver
            .abort_owner(self.admission, self.intent_consumed)
        {
            let _ = self.service.spawn_collection_driver_task(successor);
        }
    }
}

impl AppService {
    pub async fn set_interaction(&self, state: crate::InteractionState) {
        self.scheduler
            .set_interaction_mode(match state {
                crate::InteractionState::Idle => InteractionMode::Idle,
                crate::InteractionState::Active => InteractionMode::Active,
            })
            .await;
        self.coordinator.wake();
        if state == crate::InteractionState::Idle {
            self.start_derivative_driver();
        }
        let recent = self.coordinator.recent_ids().await;
        self.schedule_screen_preview_prefetch(recent).await;
    }

    pub async fn request_derivatives(
        &self,
        request: DerivativeRequest,
    ) -> Result<(), AppServiceError> {
        if request.asset_ids.len() > 250 {
            return Err(AppServiceError::InvalidLimit);
        }
        let lane = request_lane(&request);
        let ids = parse_ids(request.asset_ids)?;
        if ids.is_empty() {
            let _ = self.updates.send(WallUpdate::DerivativesReady {
                selection_id: self
                    .active_selection_id()
                    .unwrap_or_else(|| "selection-none".to_owned()),
                derivatives: Vec::new(),
                preview_counts: None,
            });
            return Ok(());
        }

        let request_selection = self.active_selection_token()?;
        let ids = {
            let state = self.state()?;
            let catalog = state.libraries.catalog();
            let stored = catalog.load_app_state()?;
            let selected_folder = stored
                .active_selection
                .ok_or(AppServiceError::UnknownAsset)?
                .relative_folder;
            validate_requested_assets(
                catalog,
                request_selection.group_id,
                stored.gallery_scope,
                &selected_folder,
                &ids,
            )?
        };
        if ids.is_empty() {
            return Ok(());
        }
        self.coordinator.ensure_selection(request_selection).await;
        if request.priority == DerivativePriority::Visible
            && request.kind == DerivativeClass::WallThumbnail
        {
            self.coordinator.invalidate_background().await;
            self.coordinator
                .rewind_collection_to_thumbnails(request_selection)
                .await;
            #[cfg(debug_assertions)]
            if let Some(hook) = self.derivative_request_test_hook.lock().await.clone() {
                if let Some(release) = hook.release {
                    let notified = release.notified();
                    tokio::pin!(notified);
                    notified.as_mut().enable();
                    hook.started.notify_one();
                    notified.await;
                } else {
                    hook.started.notify_one();
                }
            }
        }
        let derivative_ids = if request.kind == DerivativeClass::ScreenPreview {
            let (_, wall_ready) = self.ensure_wall_thumbnails(&ids, request.priority).await?;
            if wall_ready.len() != ids.len() {
                return Err(AppServiceError::DerivativeUnavailable);
            }
            wall_ready.into_iter().collect::<Vec<_>>()
        } else {
            ids.clone()
        };
        let (selection, ready, pending) =
            self.resolve_derivatives(&derivative_ids, request.kind)?;
        let mut successful_ids = ready
            .iter()
            .map(|reference| reference.asset_id.clone())
            .collect::<HashSet<_>>();
        if !ready.is_empty() {
            self.publish_derivatives_if_active(selection, ready);
        }
        let generated = self.run_derivative_jobs_publishing(pending, lane).await;
        successful_ids.extend(generated.iter().map(|reference| reference.asset_id.clone()));
        let has_unresolved = derivative_ids
            .iter()
            .any(|id| !successful_ids.contains(&id.as_uuid().hyphenated().to_string()));
        let successful_in_request = derivative_ids
            .iter()
            .copied()
            .filter(|id| successful_ids.contains(&id.as_uuid().hyphenated().to_string()))
            .collect::<Vec<_>>();
        if request.kind == DerivativeClass::WallThumbnail {
            for id in derivative_ids.iter().copied() {
                if !successful_ids.contains(&id.as_uuid().hyphenated().to_string()) {
                    let _ = self.resolve_collection_wall_outcome(selection, id);
                }
            }
        }
        if request.kind == DerivativeClass::WallThumbnail {
            for id in successful_in_request.iter().copied() {
                self.coordinator.note_recent(id).await;
            }
            if request.priority == DerivativePriority::Visible {
                // A visible wall request rewinds collection progress.  Start
                // the full-group driver again so cached screens are replayed
                // only after the rewound thumbnail phase drains; an ordinary
                // near-viewport request remains a recent-only driver.
                self.prefetch_screen_previews(successful_in_request).await;
            } else {
                self.schedule_screen_preview_prefetch(successful_in_request)
                    .await;
            }
        }
        if has_unresolved {
            return Err(AppServiceError::DerivativeUnavailable);
        }
        Ok(())
    }

    async fn ensure_wall_thumbnails(
        &self,
        ids: &[AssetId],
        priority: DerivativePriority,
    ) -> Result<(SelectionToken, HashSet<AssetId>), AppServiceError> {
        let (selection, cached, pending) =
            self.resolve_derivatives(ids, DerivativeClass::WallThumbnail)?;
        let mut ready_ids = cached
            .iter()
            .map(|reference| reference.asset_id.clone())
            .filter_map(|id| uuid::Uuid::parse_str(&id).ok())
            .map(AssetId::from_uuid)
            .collect::<HashSet<_>>();
        if !cached.is_empty() {
            self.publish_derivatives_if_active(selection, cached);
        }
        let generated = self
            .run_derivative_jobs_publishing(
                pending,
                request_lane_for(DerivativeClass::WallThumbnail, priority),
            )
            .await;
        ready_ids.extend(
            generated
                .iter()
                .filter_map(|reference| uuid::Uuid::parse_str(&reference.asset_id).ok())
                .map(AssetId::from_uuid),
        );
        for id in ids {
            if !ready_ids.contains(id) {
                let _ = self.resolve_collection_wall_outcome(selection, *id);
            }
        }
        Ok((selection, ready_ids))
    }

    fn resolve_derivatives(
        &self,
        ids: &[AssetId],
        class: DerivativeClass,
    ) -> Result<
        (
            SelectionToken,
            Vec<DerivativeReference>,
            Vec<PendingDerivative>,
        ),
        AppServiceError,
    > {
        self.resolve_derivatives_with_generation(ids, class)
    }

    fn resolve_screen_previews(
        &self,
        ids: &[AssetId],
    ) -> Result<
        (
            SelectionToken,
            Vec<DerivativeReference>,
            Vec<PendingDerivative>,
        ),
        AppServiceError,
    > {
        self.resolve_derivatives_with_generation(ids, DerivativeClass::ScreenPreview)
    }

    fn resolve_derivatives_with_generation(
        &self,
        ids: &[AssetId],
        class: DerivativeClass,
    ) -> Result<
        (
            SelectionToken,
            Vec<DerivativeReference>,
            Vec<PendingDerivative>,
        ),
        AppServiceError,
    > {
        let mut state = self.state()?;
        let stored = state.libraries.catalog().load_app_state()?;
        let selection = stored
            .active_selection
            .ok_or(AppServiceError::UnknownAsset)?;
        let group = state
            .libraries
            .catalog()
            .folder_group_for_path(selection.library_id, &selection.relative_folder)?
            .ok_or(AppServiceError::UnknownAsset)?;
        state
            .libraries
            .catalog_mut()
            .touch_folder_group(group, crate::service::unix_timestamp())?;
        let selection_token = SelectionToken {
            library_id: selection.library_id,
            group_id: group,
            epoch: state.selection_epoch,
        };
        let completion_generation = self.coordinator.commit_completion_generation();
        let library = state
            .libraries
            .catalog()
            .find_library(selection.library_id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        let root = library
            .canonical_root_key
            .to_path_buf()
            .map_err(|error| photo_catalog::CatalogError::InvalidData(error.to_string()))?;
        let mut photo_assets = Vec::with_capacity(ids.len());
        for id in ids.iter().copied() {
            let asset = state
                .libraries
                .catalog()
                .find_asset(id)?
                .ok_or(AppServiceError::UnknownAsset)?;
            if asset.folder_group_id != Some(group) {
                return Err(AppServiceError::ForeignAsset);
            }
            if asset.media_kind != MediaKind::Video {
                photo_assets.push((id, asset));
            }
        }
        let photo_ids = photo_assets.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let kind = derivative_kind_name(class);
        let records = state
            .libraries
            .catalog()
            .derivatives_for_assets(&photo_ids, kind)?;
        let prerequisite_records = match class {
            DerivativeClass::WallThumbnail => state.libraries.catalog().derivatives_for_assets(
                &photo_ids,
                derivative_kind_name(DerivativeClass::ScreenPreview),
            )?,
            DerivativeClass::ScreenPreview => state.libraries.catalog().derivatives_for_assets(
                &photo_ids,
                derivative_kind_name(DerivativeClass::WallThumbnail),
            )?,
        };
        let mut ready = Vec::new();
        let mut pending = Vec::new();
        for (id, asset) in photo_assets {
            let spec = derivative_spec(&asset, class);
            let key = DerivativeKey::compute(&spec);
            if class == DerivativeClass::ScreenPreview {
                let wall_spec = derivative_spec(&asset, DerivativeClass::WallThumbnail);
                let wall_key = DerivativeKey::compute(&wall_spec);
                let wall_ready = prerequisite_records.iter().any(|record| {
                    record.asset_id == id
                        && record.folder_group_id == group
                        && record.cache_key == wall_key.as_str()
                });
                if !wall_ready {
                    continue;
                }
            }
            let existing = records.iter().find(|record| {
                record.asset_id == id
                    && record.folder_group_id == group
                    && record.cache_key == key.as_str()
            });
            if existing.is_none()
                && state
                    .libraries
                    .catalog()
                    .find_terminal_derivative_failure(id, kind, key.as_str(), asset.availability)?
                    .is_some()
            {
                continue;
            }
            if let Some(existing) = existing {
                let catalog = state.libraries.catalog_mut();
                catalog.clear_terminal_derivative_failure(id, kind)?;
                let _ = clear_terminal_warning_if_resolved(catalog, selection.library_id, id)?;
                ready.push(reference(id, class, existing.cache_key.clone()));
            } else {
                let Some(relative) = asset.relative_path.to_path_buf().ok() else {
                    continue;
                };
                let cached_source = if class == DerivativeClass::WallThumbnail {
                    let screen_spec = derivative_spec(&asset, DerivativeClass::ScreenPreview);
                    let screen_key = DerivativeKey::compute(&screen_spec);
                    prerequisite_records
                        .iter()
                        .find(|record| {
                            record.asset_id == id
                                && record.folder_group_id == group
                                && record.cache_key == screen_key.as_str()
                        })
                        .map(|record| record.relative_cache_path.clone())
                } else {
                    None
                };
                if matches!(asset.availability, Availability::Available) || cached_source.is_some()
                {
                    pending.push(PendingDerivative {
                        id,
                        source: root.join(relative),
                        cached_source,
                        availability: asset.availability,
                        spec,
                        group,
                        class,
                        selection: selection_token,
                        completion_generation,
                    });
                }
            }
        }
        Ok((selection_token, ready, pending))
    }

    #[cfg(test)]
    async fn run_derivative_jobs(
        &self,
        pending: Vec<PendingDerivative>,
        priority: JobPriority,
    ) -> Vec<DerivativeReference> {
        let lane = pending
            .first()
            .map(|work| lane_for_priority(work.class, priority))
            .unwrap_or(crate::derivative_coordinator::WorkLane::IdlePreview);
        self.run_derivative_jobs_with_publication(pending, lane, None)
            .await
    }

    async fn run_derivative_jobs_publishing(
        &self,
        pending: Vec<PendingDerivative>,
        lane: crate::derivative_coordinator::WorkLane,
    ) -> Vec<DerivativeReference> {
        self.run_derivative_jobs_with_publication(pending, lane, None)
            .await
    }

    async fn run_collection_derivative_jobs_publishing(
        &self,
        pending: Vec<PendingDerivative>,
        lane: crate::derivative_coordinator::WorkLane,
        token: &CollectionProgressToken,
    ) -> Vec<DerivativeReference> {
        self.run_derivative_jobs_with_publication(pending, lane, Some(token))
            .await
    }

    async fn run_derivative_jobs_with_publication(
        &self,
        pending: Vec<PendingDerivative>,
        lane: crate::derivative_coordinator::WorkLane,
        collection_token: Option<&CollectionProgressToken>,
    ) -> Vec<DerivativeReference> {
        let mut receivers = Vec::with_capacity(pending.len());
        for pending in pending {
            if self.pending_has_terminal_failure(&pending).unwrap_or(false) {
                continue;
            }
            let key = WorkKey {
                selection: pending.selection,
                asset_id: pending.id,
                class: pending.class,
                cache_key: DerivativeKey::compute(&pending.spec).as_str().to_owned(),
                availability: pending.availability,
                scope: GalleryScope::IncludeSubfolders,
            };
            // Availability is part of the persisted terminal identity even
            // though it is not part of the derivative key.  A pending item
            // therefore proves that the source state changed and must clear
            // any in-memory terminal result for the old availability.  A
            // collection token is checked by the enqueue CAS below; avoid
            // mutating terminal state before that admission boundary.
            if collection_token.is_none() {
                self.coordinator.clear_terminal(&key).await;
            }
            let prerequisite = (pending.class == DerivativeClass::ScreenPreview).then(|| {
                DerivativeKey::compute(&wall_spec_from(&pending.spec))
                    .as_str()
                    .to_owned()
            });
            let receiver = if let Some(token) = collection_token {
                #[cfg(any(test, debug_assertions))]
                self.wait_for_collection_enqueue_test_gate().await;
                self.coordinator
                    .enqueue_collection_with_prerequisite_observed(
                        token,
                        key,
                        lane,
                        prerequisite,
                        Some(pending.completion_generation),
                    )
                    .await
            } else {
                self.coordinator
                    .enqueue_with_prerequisite_observed(
                        key,
                        lane,
                        prerequisite,
                        Some(pending.completion_generation),
                    )
                    .await
            };
            receivers.push(receiver);
        }
        if !receivers.is_empty() {
            #[cfg(debug_assertions)]
            if matches!(
                lane,
                crate::derivative_coordinator::WorkLane::NearWall
                    | crate::derivative_coordinator::WorkLane::ViewerPreview
                    | crate::derivative_coordinator::WorkLane::VisibleWall
            ) && let Some(hook) = self.derivative_visible_queue_test_hook.lock().await.clone()
            {
                hook.notify_one();
            }
            self.start_derivative_driver();
        }
        let mut ready = Vec::new();
        for receiver in receivers {
            if let Ok(crate::derivative_coordinator::DerivativeResult::Ready(reference)) =
                receiver.await
            {
                ready.push(reference);
            }
        }
        ready
    }

    fn pending_has_terminal_failure(
        &self,
        pending: &PendingDerivative,
    ) -> Result<bool, AppServiceError> {
        let key = DerivativeKey::compute(&pending.spec);
        Ok(self
            .state()?
            .libraries
            .catalog()
            .find_terminal_derivative_failure(
                pending.id,
                derivative_kind_name(pending.class),
                key.as_str(),
                pending.availability,
            )?
            .is_some())
    }

    fn start_derivative_driver(&self) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let service = self.clone();
        #[cfg(any(test, debug_assertions))]
        let task_guard = service.derivative_task_tracker.start();
        tokio::spawn(async move {
            #[cfg(any(test, debug_assertions))]
            let _task_guard = task_guard;
            service.drive_derivative_workers().await;
        });
    }

    async fn drive_derivative_workers(&self) {
        let _driver = self.derivative_driver.lock().await;
        let worker_count = self
            .scheduler
            .available_background_permits()
            .clamp(1, MAX_DERIVATIVE_WORKERS);
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let service = self.clone();
            workers.push(tokio::spawn(async move {
                service.drive_derivative_worker().await;
            }));
        }
        for worker in workers {
            if worker.await.is_err() {
                self.coordinator.abort_all_attempts().await;
            }
        }
    }

    async fn drive_derivative_worker(&self) {
        loop {
            let Some(ticket) = self.coordinator.next_work().await else {
                break;
            };
            let lane = self.coordinator.lane(ticket).await;
            let priority = crate::derivative_coordinator::scheduler_priority(lane);
            if should_pause(priority, self.scheduler.available_background_permits()) {
                self.coordinator.requeue(ticket).await;
                break;
            }
            let Some(key) = self.coordinator.work_key(ticket).await else {
                continue;
            };
            self.run_derivative_attempt_supervised(ticket, key).await;
        }
    }

    async fn run_derivative_attempt_supervised(&self, ticket: WorkTicket, key: WorkKey) {
        let service = self.clone();
        #[cfg(any(test, debug_assertions))]
        let attempt_guard = self.derivative_task_tracker.start();
        let attempt = tokio::spawn(async move {
            #[cfg(any(test, debug_assertions))]
            let _attempt_guard = attempt_guard;
            service.process_derivative_attempt(ticket, key).await;
        });
        #[cfg(any(test, debug_assertions))]
        {
            let attempt_abort_handle = attempt.abort_handle();
            self.register_derivative_attempt_abort_handle(ticket, attempt_abort_handle)
                .await;
        }
        let monitor_service = self.clone();
        #[cfg(any(test, debug_assertions))]
        let monitor_guard = self.derivative_task_tracker.start();
        let monitor = tokio::spawn(async move {
            #[cfg(any(test, debug_assertions))]
            let _monitor_guard = monitor_guard;
            let joined = attempt.await;
            #[cfg(any(test, debug_assertions))]
            monitor_service
                .clear_derivative_attempt_abort_handle(ticket)
                .await;
            if joined.is_err() {
                monitor_service.coordinator.abort_attempt(ticket).await;
            }
        });
        let _ = monitor.await;
    }

    async fn process_derivative_attempt(&self, ticket: WorkTicket, key: WorkKey) {
        let selection = key.selection;
        let asset_id = key.asset_id;
        let class = key.class;
        let failure_key = key.clone();
        let outcome = if self.selection_is_active(selection) {
            self.generate_derivative(ticket, key).await
        } else {
            Err(GenerationFailure {
                error: DerivativeWorkError::WorkerUnavailable,
                permit: None,
            })
        };
        let completed_by_supervisor = match outcome {
            Ok(AdmittedCommit { completion }) => {
                let _ = completion.await;
                true
            }
            Err(GenerationFailure { error, permit }) => {
                let source_key_is_current = error.scope() == DerivativeFailureScope::Asset
                    && self.derivative_key_is_current(
                        selection,
                        asset_id,
                        class,
                        &failure_key.cache_key,
                        failure_key.availability,
                    );
                let terminal = source_key_is_current
                    && self.record_terminal_derivative_failure(
                        selection,
                        asset_id,
                        class,
                        &failure_key.cache_key,
                        failure_key.availability,
                    );
                if terminal {
                    if let Some(permit) = permit {
                        self.coordinator.terminal_commit(permit).await;
                    } else {
                        self.coordinator.mark_terminal(ticket).await;
                    }
                } else {
                    if source_key_is_current || error.scope() != DerivativeFailureScope::Asset {
                        self.record_derivative_failure(selection, asset_id, class, &error);
                    }
                    if let Some(permit) = permit {
                        self.coordinator.fail_commit(permit).await;
                    } else {
                        self.coordinator.discard(ticket).await;
                    }
                }
                false
            }
        };
        #[cfg(not(any(test, debug_assertions)))]
        let _ = completed_by_supervisor;
        #[cfg(any(test, debug_assertions))]
        if !completed_by_supervisor {
            self.notify_derivative_completion_test_hook().await;
        }
    }

    fn record_derivative_failure(
        &self,
        selection: SelectionToken,
        asset_id: AssetId,
        class: DerivativeClass,
        error: &DerivativeWorkError,
    ) {
        if matches!(error, DerivativeWorkError::PreviewSuperseded)
            || matches!(error, DerivativeWorkError::WallThumbnailRequired)
        {
            return;
        }
        match error.scope() {
            DerivativeFailureScope::Asset => {
                self.record_asset_derivative_warning(selection, asset_id)
            }
            DerivativeFailureScope::CacheWide => {
                self.record_cache_derivative_warning(selection, class)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn resolve_admitted_commit(
        &self,
        permit: CommitPermit,
        selection: SelectionToken,
        asset_id: AssetId,
        class: DerivativeClass,
        cache_key: String,
        availability: Availability,
        result: Result<photo_cache::GeneratedDerivative, DerivativeWorkError>,
    ) {
        match result {
            Ok(generated) => {
                let reference = reference(asset_id, class, generated.key.as_str().into());
                self.clear_derivative_warnings(selection, asset_id, class);
                self.publish_derivatives_if_active(selection, vec![reference.clone()]);
                self.coordinator.complete_commit(permit, reference).await;
            }
            Err(error) => {
                let source_key_is_current = error.scope() == DerivativeFailureScope::Asset
                    && self.derivative_key_is_current(
                        selection,
                        asset_id,
                        class,
                        &cache_key,
                        availability,
                    );
                if source_key_is_current
                    && self.record_terminal_derivative_failure(
                        selection,
                        asset_id,
                        class,
                        &cache_key,
                        availability,
                    )
                {
                    self.coordinator.terminal_commit(permit).await;
                } else {
                    if source_key_is_current || error.scope() != DerivativeFailureScope::Asset {
                        self.record_derivative_failure(selection, asset_id, class, &error);
                    }
                    self.coordinator.fail_commit(permit).await;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_admitted_commit<F>(
        &self,
        permit: CommitPermit,
        selection: SelectionToken,
        asset_id: AssetId,
        class: DerivativeClass,
        cache_key: String,
        availability: Availability,
        operation: F,
    ) -> AdmittedCommit
    where
        F: FnOnce() -> Result<photo_cache::GeneratedDerivative, DerivativeWorkError>
            + Send
            + 'static,
    {
        let (completion_sender, completion) = tokio::sync::oneshot::channel();
        if !permit.claim_supervisor() {
            let fallback_service = self.clone();
            #[cfg(any(test, debug_assertions))]
            let fallback_guard = fallback_service.derivative_task_tracker.start();
            tokio::spawn(async move {
                #[cfg(any(test, debug_assertions))]
                let _fallback_guard = fallback_guard;
                fallback_service.coordinator.fail_commit(permit).await;
                #[cfg(any(test, debug_assertions))]
                fallback_service
                    .notify_derivative_completion_test_hook()
                    .await;
                let _ = completion_sender.send(());
            });
            return AdmittedCommit { completion };
        }
        let operation_service = self.clone();
        let fallback_permit = permit.clone();
        let operation_cache_key = cache_key.clone();
        #[cfg(any(test, debug_assertions))]
        let operation_guard = self.derivative_task_tracker.start();
        let operation_task = tokio::spawn(async move {
            #[cfg(any(test, debug_assertions))]
            let _operation_guard = operation_guard;
            let result = tokio::task::spawn_blocking(operation)
                .await
                .map_err(|_| DerivativeWorkError::WorkerUnavailable)
                .and_then(|result| result);
            operation_service
                .resolve_admitted_commit(
                    permit,
                    selection,
                    asset_id,
                    class,
                    operation_cache_key,
                    availability,
                    result,
                )
                .await;
        });
        let monitor_service = self.clone();
        #[cfg(any(test, debug_assertions))]
        let commit_monitor_guard = self.derivative_task_tracker.start();
        tokio::spawn(async move {
            #[cfg(any(test, debug_assertions))]
            let _commit_monitor_guard = commit_monitor_guard;
            if operation_task.await.is_err() {
                monitor_service
                    .resolve_admitted_commit(
                        fallback_permit,
                        selection,
                        asset_id,
                        class,
                        cache_key,
                        availability,
                        Err(DerivativeWorkError::WorkerUnavailable),
                    )
                    .await;
            }
            #[cfg(any(test, debug_assertions))]
            monitor_service
                .notify_derivative_completion_test_hook()
                .await;
            let _ = completion_sender.send(());
        });
        AdmittedCommit { completion }
    }

    async fn generate_derivative(
        &self,
        ticket: WorkTicket,
        key: WorkKey,
    ) -> Result<AdmittedCommit, GenerationFailure> {
        let pending = self
            .pending_for_key(&key)
            .map_err(|error| GenerationFailure {
                error,
                permit: None,
            })?;
        let Some(pending) = pending else {
            let error = if self.derivative_key_is_current(
                key.selection,
                key.asset_id,
                key.class,
                &key.cache_key,
                key.availability,
            ) {
                DerivativeWorkError::WorkerUnavailable
            } else {
                DerivativeWorkError::PreviewSuperseded
            };
            return Err(GenerationFailure {
                error,
                permit: None,
            });
        };
        let id = pending.id;
        let group = pending.group;
        let class = pending.class;
        let selection = pending.selection;
        let availability = pending.availability;
        #[cfg(any(test, debug_assertions))]
        self.wait_for_test_derivative_gate(id, class).await;
        if !self.selection_is_active(selection) {
            return Err(GenerationFailure {
                error: DerivativeWorkError::WorkerUnavailable,
                permit: None,
            });
        }
        match class {
            DerivativeClass::WallThumbnail => {
                let source = pending.source;
                let cached_source = pending.cached_source.clone().or(self
                    .cached_screen_preview_for_wall(id, group, &pending.spec)
                    .map_err(|error| GenerationFailure {
                        error,
                        permit: None,
                    })?);
                let spec = pending.spec;
                let permit = self
                    .coordinator
                    .admit_commit(ticket, selection, None)
                    .await
                    .ok_or_else(|| GenerationFailure {
                        error: DerivativeWorkError::PreviewSuperseded,
                        permit: None,
                    })?;
                let cache_root = self.cache_root.clone();
                let catalog_path = self.catalog_path.clone();
                let cache_key = key.cache_key.clone();
                #[cfg(any(test, debug_assertions))]
                let managed_gate = self.take_managed_commit_test_gate().await;
                #[cfg(any(test, debug_assertions))]
                let panic_hook = self.derivative_panic_in_blocking_commit_test_hook.clone();
                Ok(self.spawn_admitted_commit(
                    permit,
                    selection,
                    id,
                    class,
                    cache_key,
                    availability,
                    move || {
                        #[cfg(any(test, debug_assertions))]
                        {
                            if let Some(gate) = managed_gate {
                                gate.started.notify_one();
                                while !gate.release.load(Ordering::Acquire) {
                                    std::thread::yield_now();
                                }
                            }
                            if panic_hook.swap(false, Ordering::AcqRel) {
                                panic!("injected blocking derivative commit panic");
                            }
                        }
                        let generator = ImageDerivativeGenerator::new(&cache_root)
                            .map_err(DerivativeWorkError::from)?;
                        let generated = if let Some(cached_source) = cached_source {
                            let repair = generator
                                .generate_wall_thumbnail_from_cached_preview(&cached_source, &spec);
                            match repair {
                                Ok(generated) => Ok(generated),
                                Err(
                                    ImageDerivativeError::Io(_)
                                    | ImageDerivativeError::Decode(_)
                                    | ImageDerivativeError::Cache(photo_cache::CacheError::Io(_)),
                                ) => generator.generate(&source, &spec),
                                Err(error) => Err(error),
                            }
                        } else {
                            generator.generate(&source, &spec)
                        }
                        .map_err(DerivativeWorkError::from)?;
                        let mut catalog = Catalog::open(&catalog_path)
                            .map_err(ImageDerivativeError::from)
                            .map_err(DerivativeWorkError::from)?;
                        let catalog_started = std::time::Instant::now();
                        catalog
                            .upsert_derivative(&NewDerivative {
                                id: DerivativeId::new(),
                                asset_id: id,
                                folder_group_id: group,
                                kind: derivative_kind_name(class).into(),
                                cache_key: generated.key.as_str().into(),
                                relative_cache_path: generated.relative_path.clone(),
                                size_bytes: generated.size_bytes,
                                durable: true,
                                created_at: 0,
                            })
                            .map_err(ImageDerivativeError::from)
                            .map_err(DerivativeWorkError::from)?;
                        record_timing_stage("catalog_commit", catalog_started);
                        Ok(generated)
                    },
                ))
            }
            DerivativeClass::ScreenPreview => {
                let generator =
                    ImageDerivativeGenerator::new(&self.cache_root).map_err(|error| {
                        GenerationFailure {
                            error: error.into(),
                            permit: None,
                        }
                    })?;
                let wall_spec = wall_spec_from(&pending.spec);
                let wall_key = DerivativeKey::compute(&wall_spec);
                let wall_ready = self
                    .state()
                    .map_err(|_| GenerationFailure {
                        error: DerivativeWorkError::WorkerUnavailable,
                        permit: None,
                    })?
                    .libraries
                    .catalog()
                    .find_derivative(
                        id,
                        derivative_kind_name(DerivativeClass::WallThumbnail),
                        wall_key.as_str(),
                    )
                    .map_err(|_| GenerationFailure {
                        error: DerivativeWorkError::WorkerUnavailable,
                        permit: None,
                    })?
                    .is_some_and(|record| record.folder_group_id == group);
                if !wall_ready {
                    return Err(GenerationFailure {
                        error: DerivativeWorkError::WallThumbnailRequired,
                        permit: None,
                    });
                }
                let source = pending.source;
                let spec = pending.spec;
                #[cfg(any(test, debug_assertions))]
                let encode_counter = self.screen_preview_encode_test_counter.lock().await.clone();
                let encoded = tokio::task::spawn_blocking({
                    let generator = generator.clone();
                    let spec = spec.clone();
                    move || {
                        #[cfg(any(test, debug_assertions))]
                        if let Some(counter) = encode_counter {
                            counter.fetch_add(1, Ordering::SeqCst);
                        }
                        generator.encode_screen_preview(&source, &spec)
                    }
                })
                .await
                .map_err(|_| GenerationFailure {
                    error: DerivativeWorkError::WorkerUnavailable,
                    permit: None,
                })?
                .map_err(|error| GenerationFailure {
                    error: error.into(),
                    permit: None,
                })?;
                #[cfg(any(test, debug_assertions))]
                self.wait_for_screen_preview_post_encode_test_gate(id).await;
                if !self.selection_is_active(selection) {
                    return Err(GenerationFailure {
                        error: DerivativeWorkError::WorkerUnavailable,
                        permit: None,
                    });
                }
                let permit = self
                    .coordinator
                    .admit_commit(ticket, selection, Some(wall_key.as_str()))
                    .await
                    .ok_or_else(|| GenerationFailure {
                        error: DerivativeWorkError::PreviewSuperseded,
                        permit: None,
                    })?;
                #[cfg(any(test, debug_assertions))]
                self.wait_for_screen_preview_post_admission_test_gate(id)
                    .await;
                #[cfg(any(test, debug_assertions))]
                if self
                    .consume_derivative_panic_after_admission_test_hook()
                    .await
                {
                    panic!("injected derivative worker panic after commit admission");
                }
                let cache_budget = self.cache_budget;
                let protected = self.protected_groups.clone();
                let cache_root = self.cache_root.clone();
                let catalog_path = self.catalog_path.clone();
                let cache_key = key.cache_key.clone();
                #[cfg(any(test, debug_assertions))]
                let commit_counter = self.screen_preview_commit_test_counter.lock().await.clone();
                #[cfg(any(test, debug_assertions))]
                let managed_gate = self.take_managed_commit_test_gate().await;
                #[cfg(any(test, debug_assertions))]
                let panic_hook = self.derivative_panic_in_blocking_commit_test_hook.clone();
                Ok(self.spawn_admitted_commit(
                    permit,
                    selection,
                    id,
                    class,
                    cache_key,
                    availability,
                    move || {
                        #[cfg(any(test, debug_assertions))]
                        {
                            if let Some(gate) = managed_gate {
                                gate.started.notify_one();
                                while !gate.release.load(Ordering::Acquire) {
                                    std::thread::yield_now();
                                }
                            }
                            if panic_hook.swap(false, Ordering::AcqRel) {
                                panic!("injected blocking derivative commit panic");
                            }
                        }
                        let generator = ImageDerivativeGenerator::new(&cache_root)
                            .map_err(DerivativeWorkError::from)?;
                        let mut catalog = Catalog::open(&catalog_path)
                            .map_err(ImageDerivativeError::from)
                            .map_err(DerivativeWorkError::from)?;
                        #[cfg(any(test, debug_assertions))]
                        if let Some(counter) = commit_counter {
                            counter.fetch_add(1, Ordering::SeqCst);
                        }
                        generator
                            .commit_screen_preview(
                                encoded,
                                &spec,
                                group,
                                &mut catalog,
                                cache_budget,
                                &protected,
                            )
                            .map_err(DerivativeWorkError::from)
                    },
                ))
            }
        }
    }

    fn pending_for_key(
        &self,
        key: &WorkKey,
    ) -> Result<Option<PendingDerivative>, DerivativeWorkError> {
        let state = self
            .state()
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?;
        let stored = state
            .libraries
            .catalog()
            .load_app_state()
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?;
        let selection = stored
            .active_selection
            .ok_or(DerivativeWorkError::WorkerUnavailable)?;
        let group = state
            .libraries
            .catalog()
            .folder_group_for_path(selection.library_id, &selection.relative_folder)
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?
            .ok_or(DerivativeWorkError::WorkerUnavailable)?;
        if key.selection
            != (SelectionToken {
                library_id: selection.library_id,
                group_id: group,
                epoch: state.selection_epoch,
            })
        {
            return Ok(None);
        }
        let asset = state
            .libraries
            .catalog()
            .find_asset(key.asset_id)
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?
            .ok_or(DerivativeWorkError::WorkerUnavailable)?;
        if asset.folder_group_id != Some(group) || asset.media_kind == MediaKind::Video {
            return Ok(None);
        }
        if asset.availability != key.availability {
            return Ok(None);
        }
        let spec = derivative_spec(&asset, key.class);
        if DerivativeKey::compute(&spec).as_str() != key.cache_key {
            return Ok(None);
        }
        let library = state
            .libraries
            .catalog()
            .find_library(selection.library_id)
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?
            .ok_or(DerivativeWorkError::WorkerUnavailable)?;
        let root = library
            .canonical_root_key
            .to_path_buf()
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?;
        let relative = asset
            .relative_path
            .to_path_buf()
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?;
        let cached_source = if key.class == DerivativeClass::WallThumbnail {
            let screen_spec = derivative_spec(&asset, DerivativeClass::ScreenPreview);
            let screen_key = DerivativeKey::compute(&screen_spec);
            state
                .libraries
                .catalog()
                .find_derivative(
                    key.asset_id,
                    derivative_kind_name(DerivativeClass::ScreenPreview),
                    screen_key.as_str(),
                )
                .map_err(|_| DerivativeWorkError::WorkerUnavailable)?
                .filter(|record| record.folder_group_id == group && record.asset_id == key.asset_id)
                .map(|record| record.relative_cache_path)
        } else {
            None
        };
        if !matches!(asset.availability, Availability::Available) && cached_source.is_none() {
            return Ok(None);
        }
        Ok(Some(PendingDerivative {
            id: key.asset_id,
            source: root.join(relative),
            cached_source,
            availability: key.availability,
            spec,
            group,
            class: key.class,
            selection: key.selection,
            completion_generation: self.coordinator.commit_completion_generation(),
        }))
    }

    fn cached_screen_preview_for_wall(
        &self,
        asset_id: AssetId,
        group: FolderGroupId,
        wall_spec: &DerivativeSpec,
    ) -> Result<Option<PathBuf>, DerivativeWorkError> {
        let screen_spec = screen_spec_from(wall_spec);
        let screen_key = DerivativeKey::compute(&screen_spec);
        self.state()
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?
            .libraries
            .catalog()
            .find_derivative(
                asset_id,
                derivative_kind_name(DerivativeClass::ScreenPreview),
                screen_key.as_str(),
            )
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)
            .map(|record| {
                record
                    .filter(|record| record.folder_group_id == group)
                    .map(|record| record.relative_cache_path)
            })
    }

    #[cfg(any(test, debug_assertions))]
    async fn wait_for_test_derivative_gate(&self, asset_id: AssetId, class: DerivativeClass) {
        let gate = self.derivative_test_gate.lock().await.clone();
        if let Some(gate) = gate.filter(|gate| gate.class.is_none_or(|blocked| blocked == class)) {
            let release = gate.release.notified();
            tokio::pin!(release);
            release.as_mut().enable();
            if let Some(starts) = gate.starts {
                starts.fetch_add(1, Ordering::SeqCst);
            }
            if gate.blocked_asset.is_none() || gate.blocked_asset == Some(asset_id) {
                gate.entered.notify_one();
                release.await;
            }
        }
    }

    #[cfg(any(test, debug_assertions))]
    async fn wait_for_collection_publish_test_gate(&self) {
        if let Some(gate) = self.collection_publish_test_gate.lock().await.take() {
            let release = gate.release.notified();
            tokio::pin!(release);
            release.as_mut().enable();
            gate.entered.notify_one();
            release.await;
        }
    }

    #[cfg(any(test, debug_assertions))]
    async fn wait_for_collection_enqueue_test_gate(&self) {
        if let Some(gate) = self.collection_enqueue_test_gate.lock().await.take() {
            let release = gate.release.notified();
            tokio::pin!(release);
            release.as_mut().enable();
            gate.entered.notify_one();
            release.await;
        }
    }

    #[cfg(any(test, debug_assertions))]
    async fn wait_for_screen_preview_post_encode_test_gate(&self, asset_id: AssetId) {
        let gate = self
            .screen_preview_post_encode_test_gate
            .lock()
            .await
            .clone();
        if let Some(gate) = gate
            .filter(|gate| {
                gate.class
                    .is_none_or(|class| class == DerivativeClass::ScreenPreview)
            })
            .filter(|gate| gate.blocked_asset.is_none_or(|blocked| blocked == asset_id))
        {
            let release = gate.release.notified();
            tokio::pin!(release);
            release.as_mut().enable();
            gate.entered.notify_one();
            release.await;
        }
    }

    #[cfg(any(test, debug_assertions))]
    async fn wait_for_screen_preview_post_admission_test_gate(&self, asset_id: AssetId) {
        let gate = self
            .screen_preview_post_admission_test_gate
            .lock()
            .await
            .clone();
        if let Some(gate) = gate
            .filter(|gate| {
                gate.class
                    .is_none_or(|class| class == DerivativeClass::ScreenPreview)
            })
            .filter(|gate| gate.blocked_asset.is_none_or(|blocked| blocked == asset_id))
        {
            let release = gate.release.notified();
            tokio::pin!(release);
            release.as_mut().enable();
            gate.entered.notify_one();
            release.await;
        }
    }

    #[cfg(any(test, debug_assertions))]
    async fn notify_derivative_completion_test_hook(&self) {
        if let Some(marker) = self.derivative_completion_test_hook.lock().await.take() {
            marker.notify_one();
        }
    }

    #[cfg(any(test, debug_assertions))]
    async fn register_derivative_attempt_abort_handle(
        &self,
        ticket: WorkTicket,
        handle: tokio::task::AbortHandle,
    ) {
        *self.derivative_attempt_abort_handle.lock().await = Some((ticket, handle));
    }

    #[cfg(any(test, debug_assertions))]
    async fn clear_derivative_attempt_abort_handle(&self, ticket: WorkTicket) {
        let mut stored = self.derivative_attempt_abort_handle.lock().await;
        if stored
            .as_ref()
            .is_some_and(|(stored_ticket, _)| *stored_ticket == ticket)
        {
            *stored = None;
        }
    }

    #[cfg(any(test, debug_assertions))]
    async fn consume_derivative_panic_after_admission_test_hook(&self) -> bool {
        let mut hook = self.derivative_panic_after_admission_test_hook.lock().await;
        let panic = *hook;
        *hook = false;
        panic
    }

    #[cfg(any(test, debug_assertions))]
    async fn take_managed_commit_test_gate(&self) -> Option<crate::service::ManagedCommitTestGate> {
        self.managed_commit_test_gate.lock().await.take()
    }

    /// Installs a deterministic derivative boundary for integration tests.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_derivative_test_gate(
        &self,
        asset_id: String,
        class: DerivativeClass,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) -> Result<(), AppServiceError> {
        let asset_id = uuid::Uuid::parse_str(&asset_id)
            .map(AssetId::from_uuid)
            .map_err(|_| AppServiceError::InvalidAssetId)?;
        *self.derivative_test_gate.lock().await = Some(crate::service::DerivativeTestGate {
            blocked_asset: Some(asset_id),
            class: Some(class),
            entered,
            release,
            starts: None,
        });
        Ok(())
    }

    /// Installs a deterministic boundary immediately before a collection
    /// driver's cached publication admission.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_collection_publish_test_gate(
        &self,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self.collection_publish_test_gate.lock().await =
            Some(crate::service::CollectionTestGate { entered, release });
    }

    /// Installs a deterministic boundary immediately before a collection
    /// driver's background enqueue admission.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_collection_enqueue_test_gate(
        &self,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self.collection_enqueue_test_gate.lock().await =
            Some(crate::service::CollectionTestGate { entered, release });
    }

    /// Installs a deterministic counter at the managed screen-preview commit call.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_screen_preview_commit_test_counter(&self, counter: Arc<AtomicUsize>) {
        *self.screen_preview_commit_test_counter.lock().await = Some(counter);
    }

    /// Installs a counter at the start of screen-preview encoding.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_screen_preview_encode_test_counter(&self, counter: Arc<AtomicUsize>) {
        *self.screen_preview_encode_test_counter.lock().await = Some(counter);
    }

    /// Installs a class-wide deterministic derivative boundary for integration tests.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_derivative_test_class_gate_with_counter(
        &self,
        class: DerivativeClass,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
        starts: Arc<AtomicUsize>,
    ) {
        *self.derivative_test_gate.lock().await = Some(crate::service::DerivativeTestGate {
            blocked_asset: None,
            class: Some(class),
            entered,
            release,
            starts: Some(starts),
        });
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn wait_for_derivative_tasks_quiescent_test(&self) {
        self.derivative_task_tracker.wait_for_zero().await;
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn derivative_active_task_count_test(&self) -> usize {
        self.derivative_task_tracker.active_count()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn wait_for_collection_drivers_quiescent_test(&self) {
        self.collection_driver_tracker.wait_for_zero().await;
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn collection_driver_active_count_test(&self) -> usize {
        self.collection_driver_tracker.active_count()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn collection_driver_admitted_test(&self) -> bool {
        self.collection_driver.is_admitted()
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_collection_driver_exit_test_gate(
        &self,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self.collection_driver_exit_test_gate.lock().await =
            Some(crate::service::CollectionTestGate { entered, release });
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_collection_driver_post_take_test_gate(
        &self,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self.collection_driver_exit_test_gate.lock().await =
            Some(crate::service::CollectionTestGate { entered, release });
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn active_scan_count_test(&self) -> usize {
        usize::from(
            self.state
                .lock()
                .map(|state| state.active_scan.is_some())
                .unwrap_or(false),
        )
    }

    /// Installs a deterministic screen-preview boundary after encoding and before commit.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_screen_preview_post_encode_test_gate(
        &self,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self.screen_preview_post_encode_test_gate.lock().await =
            Some(crate::service::DerivativeTestGate {
                blocked_asset: None,
                class: Some(DerivativeClass::ScreenPreview),
                entered,
                release,
                starts: None,
            });
    }

    /// Installs a deterministic screen-preview boundary after admission and before commit.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_screen_preview_post_admission_test_gate(
        &self,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self.screen_preview_post_admission_test_gate.lock().await =
            Some(crate::service::DerivativeTestGate {
                blocked_asset: None,
                class: Some(DerivativeClass::ScreenPreview),
                entered,
                release,
                starts: None,
            });
    }

    /// Installs a one-shot marker for the next derivative attempt to finish.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_derivative_completion_test_hook(&self, marker: Arc<tokio::sync::Notify>) {
        *self.derivative_completion_test_hook.lock().await = Some(marker);
    }

    /// Aborts the currently supervised derivative attempt in deterministic tests.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn abort_derivative_attempt_for_test(&self) {
        if let Some((_, handle)) = self.derivative_attempt_abort_handle.lock().await.take() {
            handle.abort();
        }
    }

    /// Panics the currently supervised derivative attempt after admission.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_derivative_panic_after_admission_test_hook(&self) {
        *self.derivative_panic_after_admission_test_hook.lock().await = true;
    }

    /// Installs a deterministic gate at the start of admitted blocking commit work.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_managed_commit_started_test_gate(
        &self,
        started: Arc<tokio::sync::Notify>,
        release: Arc<std::sync::atomic::AtomicBool>,
    ) {
        *self.managed_commit_test_gate.lock().await =
            Some(crate::service::ManagedCommitTestGate { started, release });
    }

    /// Injects a panic from the admitted blocking commit operation.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_derivative_panic_in_blocking_commit_test_hook(&self) {
        self.derivative_panic_in_blocking_commit_test_hook
            .store(true, Ordering::Release);
    }

    /// Holds commit completion after waiter delivery and before invalidation is released.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_commit_waiter_delivery_test_gate(
        &self,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        self.coordinator
            .install_commit_waiter_delivery_test_gate(entered, release)
            .await;
    }

    /// Installs a cache budget for deterministic managed-cache tests.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn set_derivative_cache_budget_for_test(&mut self, budget: photo_cache::CacheBudget) {
        self.cache_budget = budget;
    }

    /// Installs a marker when background invalidation begins waiting on a commit.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_background_invalidation_wait_test_hook(
        &self,
        marker: Arc<tokio::sync::Notify>,
    ) {
        self.coordinator
            .install_invalidation_wait_test_hook(marker)
            .await;
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_visible_derivative_request_test_gate(
        &self,
        started: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    ) {
        *self.derivative_request_test_hook.lock().await =
            Some(crate::service::DerivativeRequestTestHook {
                started,
                release: Some(release),
            });
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_visible_derivative_queue_test_hook(
        &self,
        queued: Arc<tokio::sync::Notify>,
    ) {
        *self.derivative_visible_queue_test_hook.lock().await = Some(queued);
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn commit_screen_preview_if_active(
        &self,
        generator: ImageDerivativeGenerator,
        encoded: photo_cache::EncodedScreenPreview,
        spec: &DerivativeSpec,
        group: FolderGroupId,
        budget: photo_cache::CacheBudget,
        protected: &photo_cache::ProtectedGroups,
        selection: SelectionToken,
        catalog_path: &std::path::Path,
    ) -> Result<photo_cache::GeneratedDerivative, DerivativeWorkError> {
        if !self.selection_is_active(selection) {
            return Err(DerivativeWorkError::WorkerUnavailable);
        }
        let mut catalog = Catalog::open(catalog_path).map_err(ImageDerivativeError::from)?;
        generator
            .commit_screen_preview(encoded, spec, group, &mut catalog, budget, protected)
            .map_err(Into::into)
    }

    pub(crate) async fn prefetch_screen_previews(&self, recent: Vec<AssetId>) {
        self.schedule_screen_preview_prefetch_for_group(recent, true)
            .await;
    }

    pub(crate) fn wake_derivative_workers(&self) {
        self.coordinator.wake();
        self.start_derivative_driver();
    }

    async fn schedule_screen_preview_prefetch(&self, recent: Vec<AssetId>) {
        self.schedule_screen_preview_prefetch_for_group(recent, false)
            .await;
    }

    async fn schedule_screen_preview_prefetch_for_group(
        &self,
        recent: Vec<AssetId>,
        full_group: bool,
    ) {
        let Ok(selection) = self.active_selection_token() else {
            return;
        };
        self.coordinator.ensure_selection(selection).await;
        for id in recent.iter().copied() {
            self.coordinator.note_recent(id).await;
        }
        self.start_collection_driver(full_group, selection);
    }

    pub(crate) fn start_collection_driver(&self, full_group: bool, selection: SelectionToken) {
        #[cfg(any(test, debug_assertions))]
        let task_guard = self.collection_driver_tracker.start();
        if let Some(admission) = self.collection_driver.request(selection, full_group) {
            #[cfg(any(test, debug_assertions))]
            let _ = self.spawn_collection_driver_task_with_task_guard(admission, task_guard);
            #[cfg(not(any(test, debug_assertions)))]
            let _ = self.spawn_collection_driver_task(admission);
        } else {
            #[cfg(any(test, debug_assertions))]
            drop(task_guard);
        }
        // A recent request may be the only event that reaches this path.  Wake
        // the driver's generation wait after recording the coalesced intent so
        // it cannot sleep through a newly admitted request.
        self.coordinator.wake();
    }

    fn spawn_collection_driver_task(
        &self,
        admission: CollectionDriverAdmission,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if tokio::runtime::Handle::try_current().is_err() {
            self.collection_driver.abandon_owner(admission);
            return None;
        }
        let service = self.clone();
        let owner = CollectionDriverLifecycleOwner::new(service.clone(), admission);
        self.spawn_collection_driver_task_owner(owner, service)
    }

    #[cfg(any(test, debug_assertions))]
    fn spawn_collection_driver_task_with_task_guard(
        &self,
        admission: CollectionDriverAdmission,
        task_guard: crate::service::DerivativeTaskGuard,
    ) -> Option<tokio::task::JoinHandle<()>> {
        if tokio::runtime::Handle::try_current().is_err() {
            self.collection_driver.abandon_owner(admission);
            drop(task_guard);
            return None;
        }
        let service = self.clone();
        let owner = CollectionDriverLifecycleOwner::new_with_task_guard(
            service.clone(),
            admission,
            task_guard,
        );
        self.spawn_collection_driver_task_owner(owner, service)
    }

    fn spawn_collection_driver_task_owner(
        &self,
        owner: CollectionDriverLifecycleOwner,
        service: AppService,
    ) -> Option<tokio::task::JoinHandle<()>> {
        Some(tokio::spawn(async move {
            let mut owner = owner;
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            service.run_collection_driver(&mut owner).await;
        }))
    }

    async fn run_collection_driver(&self, owner: &mut CollectionDriverLifecycleOwner) {
        loop {
            let intent = match owner.take_pending_or_release() {
                CollectionDriverTake::Pending(intent) => intent,
                CollectionDriverTake::Released => {
                    #[cfg(debug_assertions)]
                    self.wait_for_collection_driver_exit_test_gate().await;
                    return;
                }
                CollectionDriverTake::Stale => return,
            };
            let generation = self.coordinator.background_generation().await;
            self.run_preview_prefetch_until_stable(
                Vec::new(),
                intent.full_group,
                intent.selection,
                generation,
            )
            .await;
        }
    }

    #[cfg(debug_assertions)]
    async fn wait_for_collection_driver_exit_test_gate(&self) {
        let gate = self.collection_driver_exit_test_gate.lock().await.take();
        if let Some(gate) = gate {
            let release = gate.release.notified();
            tokio::pin!(release);
            release.as_mut().enable();
            gate.entered.notify_one();
            release.await;
        }
    }

    async fn run_preview_prefetch_until_stable(
        &self,
        recent: Vec<AssetId>,
        full_group: bool,
        selection: SelectionToken,
        generation: u64,
    ) {
        let mut generation = generation;
        let mut observed_change = self.coordinator.change_generation();
        let mut retry_delay = std::time::Duration::from_millis(50);
        loop {
            match self
                .run_screen_preview_prefetch_for_generation(
                    recent.clone(),
                    full_group,
                    selection,
                    generation,
                )
                .await
            {
                PreviewPrefetchOutcome::Blocked => {
                    let wait_for_change = self.coordinator.wait_for_change_since(observed_change);
                    tokio::pin!(wait_for_change);
                    tokio::select! {
                        _ = &mut wait_for_change => {}
                        _ = tokio::time::sleep(retry_delay) => {}
                    }
                    observed_change = self.coordinator.change_generation();
                    retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(1));
                }
                PreviewPrefetchOutcome::Complete | PreviewPrefetchOutcome::Terminate => return,
                PreviewPrefetchOutcome::Stale => {
                    if !self.selection_is_active(selection) {
                        return;
                    }
                    generation = self.coordinator.background_generation().await;
                    observed_change = self.coordinator.change_generation();
                    retry_delay = std::time::Duration::from_millis(50);
                }
            }
        }
    }

    #[cfg(test)]
    async fn run_screen_preview_prefetch(&self, recent: Vec<AssetId>) {
        let Ok(selection) = self.active_selection_token() else {
            return;
        };
        let generation = self.coordinator.background_generation().await;
        let _ = self
            .run_screen_preview_prefetch_for_generation(recent, false, selection, generation)
            .await;
    }

    async fn run_screen_preview_prefetch_for_generation(
        &self,
        recent: Vec<AssetId>,
        full_group: bool,
        selection: SelectionToken,
        generation: u64,
    ) -> PreviewPrefetchOutcome {
        if self.coordinator.background_generation().await != generation
            || !self.selection_is_active(selection)
        {
            return PreviewPrefetchOutcome::Stale;
        }
        if !self.preview_work_can_continue(selection).await {
            return PreviewPrefetchOutcome::Blocked;
        }

        let mut recent_ids = Vec::with_capacity(RECENT_CAPACITY);
        let mut recent_seen = HashSet::new();
        for id in recent
            .into_iter()
            .chain(self.coordinator.recent_ids().await)
        {
            if recent_seen.insert(id) {
                recent_ids.push(id);
                if recent_ids.len() == RECENT_CAPACITY {
                    break;
                }
            }
        }
        let recent_ids = {
            let Ok(state) = self.state() else {
                return PreviewPrefetchOutcome::Terminate;
            };
            recent_ids
                .into_iter()
                .filter(|id| {
                    state
                        .libraries
                        .catalog()
                        .find_asset(*id)
                        .ok()
                        .flatten()
                        .is_some_and(|asset| {
                            asset.folder_group_id == Some(selection.group_id)
                                && asset.media_kind != MediaKind::Video
                        })
                })
                .collect::<Vec<_>>()
        };
        let recent_set = recent_ids.iter().copied().collect::<HashSet<_>>();

        let Some(collection_token) = self.coordinator.collection_progress_token(selection).await
        else {
            return PreviewPrefetchOutcome::Stale;
        };
        let phase = collection_token.phase;
        if !full_group {
            // Recent background work is a preview-only phase.  A recent
            // driver can acquire the collection mutex before the full-group
            // driver, so release it immediately while thumbnails are still
            // dormant or draining; returning Blocked here would strand the
            // full-group driver behind an endless retry loop.
            if !matches!(
                phase,
                crate::derivative_coordinator::CollectionPhase::Previews
                    | crate::derivative_coordinator::CollectionPhase::Complete
            ) {
                return PreviewPrefetchOutcome::Complete;
            }
            let wall_result = self
                .ensure_wall_thumbnails_lane(
                    &recent_ids,
                    crate::derivative_coordinator::WorkLane::NearWall,
                    Some(&collection_token),
                )
                .await;
            if !self
                .coordinator
                .collection_progress_token_is_current(&collection_token)
                .await
            {
                return PreviewPrefetchOutcome::Stale;
            }
            let Ok((wall_selection, _wall_ready, blocked)) = wall_result else {
                return PreviewPrefetchOutcome::Terminate;
            };
            if wall_selection != selection {
                return PreviewPrefetchOutcome::Stale;
            }
            if blocked {
                return PreviewPrefetchOutcome::Blocked;
            }
            let outcome = self
                .run_screen_preview_batch(
                    &recent_ids,
                    selection,
                    generation,
                    Some(&collection_token),
                )
                .await;
            if matches!(outcome, PreviewPrefetchOutcome::Complete)
                && !self
                    .coordinator
                    .remove_recent_if_collection_current(&collection_token, &recent_set)
                    .await
            {
                return PreviewPrefetchOutcome::Stale;
            }
            return outcome;
        }

        if phase == crate::derivative_coordinator::CollectionPhase::Complete {
            let outcome = self
                .run_screen_preview_batch(
                    &recent_ids,
                    selection,
                    generation,
                    Some(&collection_token),
                )
                .await;
            if matches!(outcome, PreviewPrefetchOutcome::Complete)
                && !self
                    .coordinator
                    .remove_recent_if_collection_current(&collection_token, &recent_set)
                    .await
            {
                return PreviewPrefetchOutcome::Stale;
            }
            return outcome;
        }
        if phase == crate::derivative_coordinator::CollectionPhase::Dormant {
            self.coordinator.begin_collection(selection).await;
        }

        let Some(mut token) = self.coordinator.collection_progress_token(selection).await else {
            return PreviewPrefetchOutcome::Stale;
        };
        if token.phase == crate::derivative_coordinator::CollectionPhase::Thumbnails {
            loop {
                if self.coordinator.background_generation().await != generation {
                    return PreviewPrefetchOutcome::Stale;
                }
                if !self.preview_work_can_continue(selection).await {
                    return PreviewPrefetchOutcome::Blocked;
                }
                let Ok((page_selection, page_ids, next)) =
                    self.remaining_group_ids_page(token.cursor.clone(), None)
                else {
                    return PreviewPrefetchOutcome::Terminate;
                };
                if page_selection != selection {
                    return PreviewPrefetchOutcome::Stale;
                }
                if !page_ids.is_empty() {
                    let wall_result = self
                        .ensure_wall_thumbnails_lane(
                            &page_ids,
                            crate::derivative_coordinator::WorkLane::IdleWall,
                            Some(&token),
                        )
                        .await;
                    if !self
                        .coordinator
                        .collection_progress_token_is_current(&token)
                        .await
                    {
                        return PreviewPrefetchOutcome::Stale;
                    }
                    let Ok((wall_selection, _wall_ready, blocked)) = wall_result else {
                        return PreviewPrefetchOutcome::Terminate;
                    };
                    if wall_selection != selection {
                        return PreviewPrefetchOutcome::Stale;
                    }
                    if blocked {
                        return PreviewPrefetchOutcome::Blocked;
                    }
                }
                let advanced = match next.clone() {
                    Some(next) => {
                        self.coordinator
                            .advance_collection(&token, Some(next))
                            .await
                    }
                    None => self.coordinator.finish_thumbnail_phase(&token).await,
                };
                if !advanced {
                    return PreviewPrefetchOutcome::Stale;
                }
                let Some(_next) = next else {
                    break;
                };
                token = match self.coordinator.collection_progress_token(selection).await {
                    Some(token) => token,
                    None => return PreviewPrefetchOutcome::Stale,
                };
            }
        }

        let recent_token = self
            .coordinator
            .collection_progress_token(selection)
            .await
            .filter(|token| {
                token.phase == crate::derivative_coordinator::CollectionPhase::Previews
            });
        let Some(recent_token) = recent_token else {
            return PreviewPrefetchOutcome::Stale;
        };
        let recent_outcome = self
            .run_screen_preview_batch(&recent_ids, selection, generation, Some(&recent_token))
            .await;
        if !matches!(recent_outcome, PreviewPrefetchOutcome::Complete) {
            return recent_outcome;
        }

        let Some(mut token) = self.coordinator.collection_progress_token(selection).await else {
            return PreviewPrefetchOutcome::Stale;
        };
        if token.phase != crate::derivative_coordinator::CollectionPhase::Previews {
            return if token.phase == crate::derivative_coordinator::CollectionPhase::Complete {
                PreviewPrefetchOutcome::Complete
            } else {
                PreviewPrefetchOutcome::Stale
            };
        }
        loop {
            if self.coordinator.background_generation().await != generation {
                return PreviewPrefetchOutcome::Stale;
            }
            if !self.preview_work_can_continue(selection).await {
                return PreviewPrefetchOutcome::Blocked;
            }
            let Ok((page_selection, page_ids, next)) =
                self.remaining_group_ids_page(token.cursor.clone(), Some(&recent_set))
            else {
                return PreviewPrefetchOutcome::Terminate;
            };
            if page_selection != selection {
                return PreviewPrefetchOutcome::Stale;
            }
            let Ok((resolved_selection, cached, pending)) = self.resolve_screen_previews(&page_ids)
            else {
                return PreviewPrefetchOutcome::Terminate;
            };
            if resolved_selection != selection {
                return PreviewPrefetchOutcome::Stale;
            }
            if !cached.is_empty()
                && self
                    .publish_collection_derivatives_if_current(&token, resolved_selection, cached)
                    .await
                    .is_none()
            {
                return PreviewPrefetchOutcome::Stale;
            }
            let pending_for_outcome = pending.clone();
            let _generated = self
                .run_collection_derivative_jobs_publishing(
                    pending,
                    crate::derivative_coordinator::WorkLane::IdlePreview,
                    &token,
                )
                .await;
            if !self
                .coordinator
                .collection_progress_token_is_current(&token)
                .await
            {
                return PreviewPrefetchOutcome::Stale;
            }
            if !self
                .screen_pending_complete(&pending_for_outcome)
                .unwrap_or(false)
            {
                return PreviewPrefetchOutcome::Blocked;
            }
            let advanced = match next.clone() {
                Some(next) => {
                    self.coordinator
                        .advance_collection(&token, Some(next))
                        .await
                }
                None => self.coordinator.finish_preview_phase(&token).await,
            };
            if !advanced {
                return PreviewPrefetchOutcome::Stale;
            }
            let Some(_next) = next else {
                break;
            };
            token = match self.coordinator.collection_progress_token(selection).await {
                Some(token) => token,
                None => return PreviewPrefetchOutcome::Stale,
            };
        }
        let Some(completed_token) = self.coordinator.collection_progress_token(selection).await
        else {
            return PreviewPrefetchOutcome::Stale;
        };
        if completed_token.phase != crate::derivative_coordinator::CollectionPhase::Complete
            || !self
                .coordinator
                .remove_recent_if_collection_current(&completed_token, &recent_set)
                .await
        {
            return PreviewPrefetchOutcome::Stale;
        }
        PreviewPrefetchOutcome::Complete
    }

    async fn run_screen_preview_batch(
        &self,
        ids: &[AssetId],
        selection: SelectionToken,
        generation: u64,
        collection_token: Option<&CollectionProgressToken>,
    ) -> PreviewPrefetchOutcome {
        if ids.is_empty() {
            return PreviewPrefetchOutcome::Complete;
        }
        let Ok((screen_selection, cached, pending)) = self.resolve_screen_previews(ids) else {
            return PreviewPrefetchOutcome::Terminate;
        };
        if screen_selection != selection
            || self.coordinator.background_generation().await != generation
        {
            return PreviewPrefetchOutcome::Stale;
        }
        if !cached.is_empty() {
            if let Some(token) = collection_token {
                if self
                    .publish_collection_derivatives_if_current(token, screen_selection, cached)
                    .await
                    .is_none()
                {
                    return PreviewPrefetchOutcome::Stale;
                }
            } else {
                self.publish_derivatives_if_active(screen_selection, cached);
            }
        }
        let pending_for_outcome = pending.clone();
        let _generated = if let Some(token) = collection_token {
            self.run_collection_derivative_jobs_publishing(
                pending,
                crate::derivative_coordinator::WorkLane::IdlePreview,
                token,
            )
            .await
        } else {
            self.run_derivative_jobs_publishing(
                pending,
                crate::derivative_coordinator::WorkLane::IdlePreview,
            )
            .await
        };
        if let Some(token) = collection_token
            && !self
                .coordinator
                .collection_progress_token_is_current(token)
                .await
        {
            return PreviewPrefetchOutcome::Stale;
        }
        if self
            .screen_pending_complete(&pending_for_outcome)
            .unwrap_or(false)
        {
            PreviewPrefetchOutcome::Complete
        } else {
            PreviewPrefetchOutcome::Blocked
        }
    }

    fn screen_pending_complete(
        &self,
        pending: &[PendingDerivative],
    ) -> Result<bool, AppServiceError> {
        let state = self.state()?;
        for work in pending {
            let key = DerivativeKey::compute(&work.spec);
            if state
                .libraries
                .catalog()
                .find_derivative(
                    work.id,
                    derivative_kind_name(DerivativeClass::ScreenPreview),
                    key.as_str(),
                )?
                .is_some_and(|record| record.folder_group_id == work.group)
            {
                continue;
            }
            if state
                .libraries
                .catalog()
                .find_terminal_derivative_failure(
                    work.id,
                    derivative_kind_name(DerivativeClass::ScreenPreview),
                    key.as_str(),
                    state
                        .libraries
                        .catalog()
                        .find_asset(work.id)?
                        .ok_or(AppServiceError::UnknownAsset)?
                        .availability,
                )?
                .is_some()
            {
                continue;
            }
            return Ok(false);
        }
        Ok(true)
    }

    fn resolve_collection_wall_outcome(
        &self,
        selection: SelectionToken,
        asset_id: AssetId,
    ) -> Result<bool, AppServiceError> {
        let (cache_key, availability, terminal, relative_path_valid) = {
            let state = self.state()?;
            let Some(asset) = state.libraries.catalog().find_asset(asset_id)? else {
                return Ok(false);
            };
            if asset.folder_group_id != Some(selection.group_id)
                || asset.media_kind == MediaKind::Video
            {
                return Ok(true);
            }
            let key =
                DerivativeKey::compute(&derivative_spec(&asset, DerivativeClass::WallThumbnail));
            let terminal = state
                .libraries
                .catalog()
                .find_terminal_derivative_failure(
                    asset_id,
                    derivative_kind_name(DerivativeClass::WallThumbnail),
                    key.as_str(),
                    asset.availability,
                )?
                .is_some();
            (
                key.as_str().to_owned(),
                asset.availability,
                terminal,
                asset.relative_path.to_path_buf().is_ok(),
            )
        };
        if terminal {
            return Ok(true);
        }
        if availability != Availability::Available || !relative_path_valid {
            return Ok(self.record_terminal_derivative_failure(
                selection,
                asset_id,
                DerivativeClass::WallThumbnail,
                &cache_key,
                availability,
            ));
        }
        Ok(false)
    }

    fn derivative_key_is_current(
        &self,
        selection: SelectionToken,
        asset_id: AssetId,
        class: DerivativeClass,
        cache_key: &str,
        availability: Availability,
    ) -> bool {
        self.state()
            .ok()
            .and_then(|state| {
                state
                    .libraries
                    .catalog()
                    .find_asset(asset_id)
                    .ok()
                    .flatten()
                    .filter(|asset| {
                        asset.folder_group_id == Some(selection.group_id)
                            && asset.media_kind != MediaKind::Video
                            && asset.availability == availability
                    })
                    .map(|asset| {
                        DerivativeKey::compute(&derivative_spec(&asset, class)).as_str()
                            == cache_key
                    })
            })
            .unwrap_or(false)
    }

    async fn preview_work_can_continue(&self, selection: SelectionToken) -> bool {
        if !self.selection_is_active(selection)
            || self
                .state()
                .map(|state| state.active_scan.is_some())
                .unwrap_or(true)
            || self.scheduler.available_background_permits() <= 1
        {
            return false;
        }
        self.coordinator.pending_job_count().await == 0
    }

    /// Persists a source-specific terminal outcome only after checking the
    /// current asset signature and availability.  The check is deliberately
    /// done at the point of publication so a stale worker cannot poison a
    /// newer source version.
    fn record_terminal_derivative_failure(
        &self,
        selection: SelectionToken,
        asset_id: AssetId,
        class: DerivativeClass,
        cache_key: &str,
        expected_availability: Availability,
    ) -> bool {
        let result = self.state().and_then(|mut state| {
            if state.selection_epoch != selection.epoch
                || state.protected_group != Some(selection.group_id)
            {
                return Ok(false);
            }
            let asset = state
                .libraries
                .catalog()
                .find_asset(asset_id)?
                .filter(|asset| {
                    asset.folder_group_id == Some(selection.group_id)
                        && asset.media_kind != MediaKind::Video
                });
            let Some(asset) = asset else {
                return Ok(false);
            };
            if asset.availability != expected_availability {
                return Ok(false);
            }
            let current_key = DerivativeKey::compute(&derivative_spec(&asset, class));
            if current_key.as_str() != cache_key {
                return Ok(false);
            }
            let kind = derivative_kind_name(class);
            state
                .libraries
                .catalog_mut()
                .record_terminal_derivative_failure(&TerminalDerivativeFailure {
                    asset_id,
                    kind: kind.to_owned(),
                    cache_key: cache_key.to_owned(),
                    availability: asset.availability,
                    failure_code: TERMINAL_ASSET_DERIVATIVE_WARNING.to_owned(),
                    occurred_at: crate::service::unix_timestamp(),
                })?;
            state.libraries.catalog_mut().clear_warning(
                selection.library_id,
                Some(asset_id),
                ASSET_DERIVATIVE_WARNING,
            )?;
            state.libraries.catalog_mut().clear_warning(
                selection.library_id,
                Some(asset_id),
                TERMINAL_ASSET_DERIVATIVE_WARNING,
            )?;
            let inserted = state.libraries.catalog_mut().record_warning_once(
                &photo_catalog::CatalogWarningRecord {
                    library_id: selection.library_id,
                    asset_id: Some(asset_id),
                    code: TERMINAL_ASSET_DERIVATIVE_WARNING.to_owned(),
                    message: "A preview could not be generated for this source version. It will retry when the source changes or becomes available.".to_owned(),
                },
            )?;
            if inserted {
                let _ = self.updates.send(WallUpdate::Warning {
                    selection_id: selection.selection_id(),
                    source_id: selection.library_id.as_uuid().hyphenated().to_string(),
                    asset_id: Some(asset_id.as_uuid().hyphenated().to_string()),
                    warning: crate::WallWarningState {
                        code: "derivativeUnavailable".to_owned(),
                        retryable: false,
                    },
                });
            }
            Ok(true)
        });
        matches!(result, Ok(true))
    }

    async fn ensure_wall_thumbnails_lane(
        &self,
        ids: &[AssetId],
        lane: crate::derivative_coordinator::WorkLane,
        collection_token: Option<&CollectionProgressToken>,
    ) -> Result<(SelectionToken, HashSet<AssetId>, bool), AppServiceError> {
        let (selection, cached, pending) =
            self.resolve_derivatives(ids, DerivativeClass::WallThumbnail)?;
        let mut ready_ids = cached
            .iter()
            .map(|reference| reference.asset_id.clone())
            .filter_map(|id| uuid::Uuid::parse_str(&id).ok())
            .map(AssetId::from_uuid)
            .collect::<HashSet<_>>();
        if !cached.is_empty() {
            if let Some(token) = collection_token {
                if self
                    .publish_collection_derivatives_if_current(token, selection, cached)
                    .await
                    .is_none()
                {
                    return Err(AppServiceError::DerivativeUnavailable);
                }
            } else {
                self.publish_derivatives_if_active(selection, cached);
            }
        }
        let generated = if let Some(token) = collection_token {
            self.run_collection_derivative_jobs_publishing(pending, lane, token)
                .await
        } else {
            self.run_derivative_jobs_publishing(pending, lane).await
        };
        if let Some(token) = collection_token
            && !self
                .coordinator
                .collection_progress_token_is_current(token)
                .await
        {
            return Err(AppServiceError::DerivativeUnavailable);
        }
        ready_ids.extend(
            generated
                .iter()
                .filter_map(|reference| uuid::Uuid::parse_str(&reference.asset_id).ok())
                .map(AssetId::from_uuid),
        );
        let mut blocked = false;
        for id in ids {
            if ready_ids.contains(id) {
                continue;
            }
            if !self.resolve_collection_wall_outcome(selection, *id)? {
                blocked = true;
            }
        }
        Ok((selection, ready_ids, blocked))
    }

    fn remaining_group_ids_page(
        &self,
        cursor: Option<photo_catalog::WallCursorKey>,
        excluded: Option<&HashSet<AssetId>>,
    ) -> Result<
        (
            SelectionToken,
            Vec<AssetId>,
            Option<photo_catalog::WallCursorKey>,
        ),
        AppServiceError,
    > {
        let state = self.state()?;
        let stored = state.libraries.catalog().load_app_state()?;
        let selection = stored
            .active_selection
            .ok_or(AppServiceError::UnknownAsset)?;
        let group = state
            .libraries
            .catalog()
            .folder_group_for_path(selection.library_id, &selection.relative_folder)?
            .ok_or(AppServiceError::UnknownAsset)?;
        let settled = state
            .libraries
            .catalog()
            .has_completed_generation_for_group(selection.library_id, group)?;
        let order = if settled {
            WallOrder::CapturedAscending
        } else {
            WallOrder::Provisional
        };
        let page = state.libraries.catalog().photo_asset_ids_page_scoped(
            group,
            stored.gallery_scope,
            order,
            cursor,
            250,
        )?;
        #[cfg(debug_assertions)]
        let page_size = page.items.len();
        #[cfg(debug_assertions)]
        self.coordinator.note_loaded_page(page_size);
        let items = match excluded {
            Some(excluded) => page
                .items
                .into_iter()
                .filter(|id| !excluded.contains(id))
                .collect(),
            None => page.items,
        };
        Ok((
            SelectionToken {
                library_id: selection.library_id,
                group_id: group,
                epoch: state.selection_epoch,
            },
            items,
            page.next,
        ))
    }

    pub(crate) fn active_selection_token(&self) -> Result<SelectionToken, AppServiceError> {
        let state = self.state()?;
        let selection = state
            .libraries
            .catalog()
            .load_app_state()?
            .active_selection
            .ok_or(AppServiceError::UnknownAsset)?;
        let group = state
            .libraries
            .catalog()
            .folder_group_for_path(selection.library_id, &selection.relative_folder)?
            .ok_or(AppServiceError::UnknownAsset)?;
        Ok(SelectionToken {
            library_id: selection.library_id,
            group_id: group,
            epoch: state.selection_epoch,
        })
    }

    fn selection_is_active(&self, selection: SelectionToken) -> bool {
        self.state()
            .map(|state| {
                state.selection_epoch == selection.epoch
                    && state.protected_group == Some(selection.group_id)
            })
            .unwrap_or(false)
    }

    fn publish_derivatives_if_active(
        &self,
        selection: SelectionToken,
        derivatives: Vec<DerivativeReference>,
    ) -> bool {
        let Ok(state) = self.state.lock() else {
            return false;
        };
        if state.selection_epoch != selection.epoch
            || state.protected_group != Some(selection.group_id)
        {
            return false;
        }
        let preview_counts = state
            .libraries
            .catalog()
            .load_app_state()
            .and_then(|stored| {
                state
                    .libraries
                    .catalog()
                    .wall_preview_counts_scoped(selection.group_id, stored.gallery_scope)
            })
            .ok()
            .map(|counts| crate::WallPreviewCounts {
                wall_ready: counts.wall_ready,
                screen_ready: counts.screen_ready,
            });
        let publication_started = std::time::Instant::now();
        let result = self.updates.send(WallUpdate::DerivativesReady {
            selection_id: selection.selection_id(),
            derivatives,
            preview_counts,
        });
        record_timing_stage("derivative_publication", publication_started);
        result.is_ok()
    }

    async fn publish_collection_derivatives_if_current(
        &self,
        token: &CollectionProgressToken,
        selection: SelectionToken,
        derivatives: Vec<DerivativeReference>,
    ) -> Option<bool> {
        #[cfg(any(test, debug_assertions))]
        if derivatives
            .iter()
            .any(|derivative| derivative.kind == DerivativeClass::ScreenPreview)
        {
            self.wait_for_collection_publish_test_gate().await;
        }
        self.coordinator
            .collection_side_effect_if_current(token, || {
                self.publish_derivatives_if_active(selection, derivatives)
            })
            .await
    }

    fn record_asset_derivative_warning(&self, selection: SelectionToken, asset_id: AssetId) {
        let warning = crate::WallWarningState {
            code: "derivativeUnavailable".to_owned(),
            retryable: true,
        };
        let _ = self.state().and_then(|mut state| {
            if state.selection_epoch != selection.epoch
                || state.protected_group != Some(selection.group_id)
            {
                return Ok(());
            }
            let inserted = state.libraries.catalog_mut().record_warning_once(
                &photo_catalog::CatalogWarningRecord {
                    library_id: selection.library_id,
                    asset_id: Some(asset_id),
                    code: ASSET_DERIVATIVE_WARNING.to_owned(),
                    message: "A cached preview could not be generated. The app can retry it."
                        .to_owned(),
                },
            )?;
            if inserted {
                let _ = self.updates.send(WallUpdate::Warning {
                    selection_id: selection.selection_id(),
                    source_id: selection.library_id.as_uuid().hyphenated().to_string(),
                    asset_id: Some(asset_id.as_uuid().hyphenated().to_string()),
                    warning,
                });
            }
            Ok(())
        });
    }

    fn record_cache_derivative_warning(&self, selection: SelectionToken, class: DerivativeClass) {
        let (catalog_code, public_code, message) = cache_warning_identity(class);
        let warning = crate::WallWarningState {
            code: public_code.to_owned(),
            retryable: true,
        };
        let _ = self.state().and_then(|mut state| {
            if state.selection_epoch != selection.epoch
                || state.protected_group != Some(selection.group_id)
            {
                return Ok(());
            }
            let should_publish = match class {
                DerivativeClass::WallThumbnail => {
                    state.published_wall_cache_warning != Some(selection)
                }
                DerivativeClass::ScreenPreview => {
                    state.published_screen_cache_warning != Some(selection)
                }
            };
            if should_publish {
                state.libraries.catalog_mut().record_warning_once(
                    &photo_catalog::CatalogWarningRecord {
                        library_id: selection.library_id,
                        asset_id: None,
                        code: catalog_code.to_owned(),
                        message: message.to_owned(),
                    },
                )?;
                let _ = self.updates.send(WallUpdate::Warning {
                    selection_id: selection.selection_id(),
                    source_id: selection.library_id.as_uuid().hyphenated().to_string(),
                    asset_id: None,
                    warning,
                });
                match class {
                    DerivativeClass::WallThumbnail => {
                        state.published_wall_cache_warning = Some(selection)
                    }
                    DerivativeClass::ScreenPreview => {
                        state.published_screen_cache_warning = Some(selection)
                    }
                }
            }
            Ok(())
        });
    }

    fn clear_derivative_warnings(
        &self,
        selection: SelectionToken,
        asset_id: AssetId,
        class: DerivativeClass,
    ) {
        let (catalog_code, public_code, _) = cache_warning_identity(class);
        let _ = self.state().and_then(|mut state| {
            if state.selection_epoch != selection.epoch
                || state.protected_group != Some(selection.group_id)
            {
                return Ok(());
            }
            let asset_warning_removed = state.libraries.catalog_mut().clear_warning(
                selection.library_id,
                Some(asset_id),
                ASSET_DERIVATIVE_WARNING,
            )?;
            let cache_warning_removed = state.libraries.catalog_mut().clear_warning(
                selection.library_id,
                None,
                catalog_code,
            )?;
            let catalog = state.libraries.catalog_mut();
            catalog.clear_terminal_derivative_failure(asset_id, derivative_kind_name(class))?;
            let has_current_terminal_failure = match catalog.find_asset(asset_id)? {
                Some(asset) => has_current_terminal_derivative_failure(catalog, &asset)?,
                None => true,
            };
            let terminal_warning_removed = if has_current_terminal_failure {
                0
            } else {
                catalog.clear_warning(
                    selection.library_id,
                    Some(asset_id),
                    TERMINAL_ASSET_DERIVATIVE_WARNING,
                )?
            };
            let source_id = selection.library_id.as_uuid().hyphenated().to_string();
            if !has_current_terminal_failure && asset_warning_removed + terminal_warning_removed > 0
            {
                let _ = self.updates.send(WallUpdate::WarningCleared {
                    selection_id: selection.selection_id(),
                    source_id: source_id.clone(),
                    asset_id: Some(asset_id.as_uuid().hyphenated().to_string()),
                    code: "derivativeUnavailable".to_owned(),
                });
            }
            if cache_warning_removed > 0 {
                match class {
                    DerivativeClass::WallThumbnail => {
                        state.published_wall_cache_warning = None;
                    }
                    DerivativeClass::ScreenPreview => {
                        state.published_screen_cache_warning = None;
                    }
                }
                let _ = self.updates.send(WallUpdate::WarningCleared {
                    selection_id: selection.selection_id(),
                    source_id,
                    asset_id: None,
                    code: public_code.to_owned(),
                });
            }
            Ok(())
        });
    }
}

fn cache_warning_identity(class: DerivativeClass) -> (&'static str, &'static str, &'static str) {
    match class {
        DerivativeClass::WallThumbnail => (
            WALL_CACHE_DERIVATIVE_WARNING,
            "wallThumbnailCacheUnavailable",
            "The thumbnail cache is unavailable. Browsing can continue while the app retries it.",
        ),
        DerivativeClass::ScreenPreview => (
            SCREEN_CACHE_DERIVATIVE_WARNING,
            "screenPreviewCacheUnavailable",
            "The preview cache is unavailable. Browsing can continue while the app retries it.",
        ),
    }
}

fn has_current_terminal_derivative_failure(
    catalog: &Catalog,
    asset: &photo_catalog::AssetRecord,
) -> Result<bool, photo_catalog::CatalogError> {
    if asset.media_kind == MediaKind::Video || asset.folder_group_id.is_none() {
        return Ok(false);
    }
    for class in [
        DerivativeClass::WallThumbnail,
        DerivativeClass::ScreenPreview,
    ] {
        let key = DerivativeKey::compute(&derivative_spec(asset, class));
        if catalog
            .find_terminal_derivative_failure(
                asset.id,
                derivative_kind_name(class),
                key.as_str(),
                asset.availability,
            )?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn clear_terminal_warning_if_resolved(
    catalog: &mut Catalog,
    library_id: photo_domain::LibraryId,
    asset_id: AssetId,
) -> Result<u64, photo_catalog::CatalogError> {
    let Some(asset) = catalog.find_asset(asset_id)? else {
        return Ok(0);
    };
    if has_current_terminal_derivative_failure(catalog, &asset)? {
        return Ok(0);
    }
    catalog.clear_warning(
        library_id,
        Some(asset_id),
        TERMINAL_ASSET_DERIVATIVE_WARNING,
    )
}

fn parse_ids(raw_ids: Vec<String>) -> Result<Vec<AssetId>, AppServiceError> {
    let mut ids = Vec::with_capacity(raw_ids.len());
    let mut seen = HashSet::new();
    for raw in raw_ids {
        let uuid = uuid::Uuid::parse_str(&raw).map_err(|_| AppServiceError::InvalidAssetId)?;
        let id = AssetId::from_uuid(uuid);
        if seen.insert(id) {
            ids.push(id);
        }
    }
    Ok(ids)
}

fn validate_requested_assets(
    catalog: &Catalog,
    group: FolderGroupId,
    scope: GalleryScope,
    selected_folder: &RelativePathKey,
    ids: &[AssetId],
) -> Result<Vec<AssetId>, AppServiceError> {
    let selected_folder = selected_folder
        .to_path_buf()
        .map_err(|_| AppServiceError::ForeignAsset)?;
    let mut photo_ids = Vec::with_capacity(ids.len());
    for id in ids {
        let asset = catalog
            .find_asset(*id)?
            .ok_or(AppServiceError::UnknownAsset)?;
        if asset.folder_group_id != Some(group) {
            return Err(AppServiceError::ForeignAsset);
        }
        if asset.media_kind == MediaKind::Video {
            return Err(AppServiceError::DerivativeUnavailable);
        }
        if scope == GalleryScope::CurrentFolder {
            let asset_path = asset
                .relative_path
                .to_path_buf()
                .map_err(|_| AppServiceError::ForeignAsset)?;
            if asset_path.parent() != Some(selected_folder.as_path()) {
                return Err(AppServiceError::ForeignAsset);
            }
        }
        photo_ids.push(*id);
    }
    Ok(photo_ids)
}

fn derivative_spec(asset: &photo_catalog::AssetRecord, class: DerivativeClass) -> DerivativeSpec {
    let (kind, edge) = match class {
        DerivativeClass::WallThumbnail => (DerivativeKind::WallThumbnail, 1024),
        DerivativeClass::ScreenPreview => (DerivativeKind::ScreenPreview, 4096),
    };
    DerivativeSpec {
        asset_id: asset.id,
        signature: asset.signature,
        orientation: asset.orientation.unwrap_or(1),
        kind,
        decoder_version: DECODER_VERSION.into(),
        colour_space: "srgb".into(),
        target: DerivativeTarget::LongEdge(edge),
    }
}

fn wall_spec_from(spec: &DerivativeSpec) -> DerivativeSpec {
    DerivativeSpec {
        kind: DerivativeKind::WallThumbnail,
        target: DerivativeTarget::LongEdge(1024),
        ..spec.clone()
    }
}

fn screen_spec_from(spec: &DerivativeSpec) -> DerivativeSpec {
    DerivativeSpec {
        kind: DerivativeKind::ScreenPreview,
        target: DerivativeTarget::LongEdge(4096),
        ..spec.clone()
    }
}

fn derivative_kind_name(class: DerivativeClass) -> &'static str {
    match class {
        DerivativeClass::WallThumbnail => "wall_thumbnail",
        DerivativeClass::ScreenPreview => "screen_preview",
    }
}

fn request_lane(request: &DerivativeRequest) -> crate::derivative_coordinator::WorkLane {
    request_lane_for(request.kind, request.priority)
}

fn request_lane_for(
    class: DerivativeClass,
    priority: DerivativePriority,
) -> crate::derivative_coordinator::WorkLane {
    use crate::derivative_coordinator::WorkLane;

    match (class, priority) {
        (DerivativeClass::WallThumbnail, DerivativePriority::Visible) => WorkLane::VisibleWall,
        (DerivativeClass::WallThumbnail, DerivativePriority::NearViewport) => WorkLane::NearWall,
        (DerivativeClass::ScreenPreview, DerivativePriority::Visible) => WorkLane::ViewerPreview,
        (DerivativeClass::ScreenPreview, DerivativePriority::NearViewport) => WorkLane::IdlePreview,
    }
}

#[cfg(test)]
fn lane_for_priority(
    class: DerivativeClass,
    priority: JobPriority,
) -> crate::derivative_coordinator::WorkLane {
    match class {
        DerivativeClass::WallThumbnail => match priority {
            JobPriority::Visible => crate::derivative_coordinator::WorkLane::VisibleWall,
            JobPriority::NearViewport => crate::derivative_coordinator::WorkLane::NearWall,
            _ => crate::derivative_coordinator::WorkLane::IdleWall,
        },
        DerivativeClass::ScreenPreview => match priority {
            JobPriority::Visible | JobPriority::ViewerPreview => {
                crate::derivative_coordinator::WorkLane::ViewerPreview
            }
            _ => crate::derivative_coordinator::WorkLane::IdlePreview,
        },
    }
}

fn should_pause(priority: JobPriority, background_permits: usize) -> bool {
    matches!(
        priority,
        JobPriority::IdleLibrary | JobPriority::OpenCollection
    ) && background_permits <= 1
}

fn reference(id: AssetId, kind: DerivativeClass, key: String) -> DerivativeReference {
    DerivativeReference {
        asset_id: id.as_uuid().hyphenated().to_string(),
        kind,
        key,
    }
}

#[cfg(test)]
mod tests {
    #[cfg(debug_assertions)]
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::path::Path;
    #[cfg(debug_assertions)]
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    use image::{ImageBuffer, Rgb};
    #[cfg(debug_assertions)]
    use photo_cache::DerivativeKey;
    use photo_cache::{
        CacheBudget, DerivativeKind, DerivativeSpec, DerivativeTarget, EncodedScreenPreview,
        ImageDerivativeGenerator,
    };
    #[cfg(debug_assertions)]
    use photo_catalog::NewDerivative;
    #[cfg(debug_assertions)]
    use photo_catalog::{
        AssetShapeUpdate, CatalogIndexRecord, CatalogWarningRecord, ShapeStatus,
        TerminalDerivativeFailure,
    };
    use photo_catalog::{Catalog, NewAsset};
    use photo_domain::{AssetId, FileSignature, GalleryScope, MediaKind, RelativePathKey};
    #[cfg(debug_assertions)]
    use photo_domain::{FolderGroupId, LibraryId};
    use photo_indexer::{JobPriority, MetadataReader};
    use photo_metadata::{MetadataBundle, MetadataReadWarning};

    use crate::service::DerivativeTestGate;
    #[cfg(debug_assertions)]
    use crate::service::{CollectionDriverControl, CollectionDriverTake, SelectionToken};
    use crate::{
        AppConfig, AppService, AppServiceError, DerivativeClass, DerivativePriority,
        DerivativeRequest, WallUpdate,
    };

    #[cfg(debug_assertions)]
    use super::CollectionDriverLifecycleOwner;
    use super::DerivativeWorkError;
    #[cfg(debug_assertions)]
    use super::PreviewPrefetchOutcome;

    struct BlockingReader {
        gate: Arc<(Mutex<bool>, Condvar)>,
        entered: Arc<tokio::sync::Notify>,
    }

    #[tokio::test]
    async fn collection_prefetch_page_honors_current_folder_scope() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(source.join("child")).unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config).unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        let mut ids = Vec::new();
        {
            let mut state = service.state().unwrap();
            for path in ["direct.jpg", "child/nested.jpg"] {
                let relative = RelativePathKey::from_relative_path(Path::new(path)).unwrap();
                let mut asset =
                    NewAsset::minimal(selection.library_id, relative, path, MediaKind::Jpeg, 1);
                asset.folder_group_id = Some(selection.group_id);
                ids.push(asset.id);
                state
                    .libraries
                    .catalog_mut()
                    .apply_index_batch(&[
                        CatalogIndexRecord::Discovered(asset.clone()),
                        CatalogIndexRecord::Shaped(AssetShapeUpdate {
                            asset_id: asset.id,
                            width: 16,
                            height: 9,
                            orientation: Some(1),
                            representative_rgb: None,
                            shape_status: ShapeStatus::Ready,
                        }),
                    ])
                    .unwrap();
            }
        }
        service
            .update_gallery_scope(GalleryScope::CurrentFolder)
            .await
            .unwrap();

        let (_, page, next) = service.remaining_group_ids_page(None, None).unwrap();

        assert_eq!(page, [ids[0]]);
        assert!(next.is_some());
    }

    #[tokio::test]
    async fn gallery_scope_change_invalidates_old_collection_progress() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        service.coordinator.reset_selection(selection).await;
        service.coordinator.begin_collection(selection).await;
        let old_progress = service
            .coordinator
            .collection_progress_token(selection)
            .await
            .unwrap();

        service
            .update_gallery_scope(GalleryScope::CurrentFolder)
            .await
            .unwrap();

        assert!(
            !service
                .coordinator
                .collection_progress_token_is_current(&old_progress)
                .await
        );
    }

    #[tokio::test]
    async fn current_folder_scope_rejects_a_nested_foreground_derivative_request() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(source.join("child")).unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        let nested_id = {
            let relative =
                RelativePathKey::from_relative_path(Path::new("child/nested.jpg")).unwrap();
            let mut asset = NewAsset::minimal(
                selection.library_id,
                relative,
                "child/nested.jpg",
                MediaKind::Jpeg,
                1,
            );
            asset.folder_group_id = Some(selection.group_id);
            let id = asset.id;
            service
                .state()
                .unwrap()
                .libraries
                .catalog_mut()
                .apply_index_batch(&[
                    CatalogIndexRecord::Discovered(asset),
                    CatalogIndexRecord::Shaped(AssetShapeUpdate {
                        asset_id: id,
                        width: 16,
                        height: 9,
                        orientation: Some(1),
                        representative_rgb: None,
                        shape_status: ShapeStatus::Ready,
                    }),
                ])
                .unwrap();
            id
        };
        service
            .update_gallery_scope(GalleryScope::CurrentFolder)
            .await
            .unwrap();

        let error = service
            .request_derivatives(DerivativeRequest {
                asset_ids: vec![nested_id.as_uuid().hyphenated().to_string()],
                kind: DerivativeClass::WallThumbnail,
                priority: DerivativePriority::Visible,
            })
            .await
            .unwrap_err();

        assert!(matches!(error, AppServiceError::ForeignAsset));
    }

    impl MetadataReader for BlockingReader {
        fn read(
            &self,
            _media_path: &Path,
            _sidecar_path: Option<&Path>,
        ) -> Result<MetadataBundle, MetadataReadWarning> {
            self.entered.notify_one();
            let (released, changed) = &*self.gate;
            let mut released = released.lock().unwrap();
            while !*released {
                released = changed.wait(released).unwrap();
            }
            Ok(MetadataBundle::default())
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn healthy_visible_derivative_publishes_before_a_blocked_one() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let blocked_path = source.join("blocked.jpg");
        let healthy_path = source.join("healthy.jpg");
        ImageBuffer::from_pixel(32, 24, Rgb([220_u8, 10, 20]))
            .save(&blocked_path)
            .unwrap();
        ImageBuffer::from_pixel(32, 24, Rgb([10_u8, 180, 40]))
            .save(&healthy_path)
            .unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config).unwrap();
        let mut updates = service.subscribe_wall_updates();
        service.start_scan(&source).await.unwrap();
        receive_until(&mut updates, |event| {
            matches!(event, WallUpdate::MetadataSettled { .. })
        })
        .await;
        let page = service
            .query_wall(crate::WallQueryRequest::oldest_first())
            .await
            .unwrap();
        let blocked_id = page
            .items
            .iter()
            .find(|asset| asset.display_name == "blocked.jpg")
            .unwrap()
            .id
            .clone();
        let healthy_id = page
            .items
            .iter()
            .find(|asset| asset.display_name == "healthy.jpg")
            .unwrap()
            .id
            .clone();
        let blocked_before = std::fs::read(&blocked_path).unwrap();
        let healthy_before = std::fs::read(&healthy_path).unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        *service.derivative_test_gate.lock().await = Some(DerivativeTestGate {
            blocked_asset: Some(AssetId::from_uuid(
                uuid::Uuid::parse_str(&blocked_id).unwrap(),
            )),
            class: None,
            entered: entered.clone(),
            release: release.clone(),
            starts: None,
        });
        let entered_wait = entered.notified();
        let request_service = service.clone();
        let healthy_id_for_request = healthy_id.clone();
        let request = tokio::spawn(async move {
            request_service
                .request_derivatives(crate::DerivativeRequest::visible(vec![
                    blocked_id,
                    healthy_id_for_request,
                ]))
                .await
        });
        entered_wait.await;
        let healthy_event = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            receive_until(&mut updates, |event| {
                matches!(event, WallUpdate::DerivativesReady { derivatives, .. }
                if derivatives.iter().any(|item| {
                    item.kind == DerivativeClass::WallThumbnail && item.asset_id == healthy_id
                }))
            }),
        )
        .await
        .expect("healthy visible derivative should publish while blocked work is held");
        assert!(matches!(
            healthy_event,
            WallUpdate::DerivativesReady { ref derivatives, .. }
                if derivatives.iter().any(|item| item.asset_id == healthy_id)
        ));
        release.notify_one();
        request.await.unwrap().unwrap();
        assert_eq!(std::fs::read(&blocked_path).unwrap(), blocked_before);
        assert_eq!(std::fs::read(&healthy_path).unwrap(), healthy_before);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn scan_completion_wakes_prefetch_when_coordinator_snapshot_is_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        ImageBuffer::from_pixel(8, 8, Rgb([20_u8, 40, 60]))
            .save(source.join("photo.jpg"))
            .unwrap();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        let entered = Arc::new(tokio::sync::Notify::new());
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open_with_reader(
            config,
            Arc::new(BlockingReader {
                gate: gate.clone(),
                entered: entered.clone(),
            }),
        )
        .unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        service.coordinator.ensure_selection(selection).await;
        let mut updates = service.subscribe_wall_updates();
        let entered_wait = entered.notified();
        service.start_selected_scan(selection).await.unwrap();
        tokio::time::timeout(Duration::from_secs(2), entered_wait)
            .await
            .expect("scan did not reach the blocking metadata reader");

        let generation = service.coordinator.background_generation().await;
        let change_generation = service.coordinator.change_generation();
        let wait_entered = Arc::new(tokio::sync::Notify::new());
        let wait_entered_wait = wait_entered.notified();
        let scan_wake = Arc::new(tokio::sync::Notify::new());
        let scan_wake_wait = scan_wake.notified();
        service
            .coordinator
            .install_wait_test_hook(wait_entered.clone())
            .await;
        service.install_scan_completion_wake_test_hook(selection, scan_wake.clone());
        let prefetch_service = service.clone();
        let prefetch = tokio::spawn(async move {
            prefetch_service
                .run_preview_prefetch_until_stable(Vec::new(), true, selection, generation)
                .await;
        });
        tokio::time::timeout(Duration::from_secs(2), wait_entered_wait)
            .await
            .expect("prefetch did not reach its generation wait barrier");
        assert!(
            !prefetch.is_finished(),
            "prefetch did not block at the generation wait barrier"
        );
        assert_eq!(service.coordinator.selection().await, Some(selection));
        assert_eq!(
            service.coordinator.background_generation().await,
            generation
        );
        assert_eq!(service.coordinator.pending_job_count().await, 0);
        assert_eq!(service.scheduler.available_background_permits(), 4);

        {
            let (released, changed) = &*gate;
            *released.lock().unwrap() = true;
            changed.notify_all();
        }
        tokio::time::timeout(
            Duration::from_secs(5),
            receive_until(&mut updates, |event| {
                matches!(event, WallUpdate::MetadataSettled { .. })
            }),
        )
        .await
        .expect("scan did not complete after releasing the metadata reader");
        tokio::time::timeout(Duration::from_secs(5), scan_wake_wait)
            .await
            .expect("scan completion did not reach its post-wake marker");
        assert!(
            service.coordinator.change_generation() > change_generation,
            "scan completion did not record its explicit coordinator wake"
        );
        tokio::time::timeout(Duration::from_secs(5), prefetch)
            .await
            .expect("prefetch was not woken by scan completion")
            .expect("prefetch task panicked");
    }

    #[cfg(debug_assertions)]
    async fn collection_gap_fixture(
        with_screen: bool,
    ) -> (
        tempfile::TempDir,
        AppService,
        SelectionToken,
        AssetId,
        usize,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let image_path = source.join("photo.jpg");
        ImageBuffer::from_pixel(8, 8, Rgb([20_u8, 40, 60]))
            .save(&image_path)
            .unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config).unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        service.coordinator.ensure_selection(selection).await;
        let relative = RelativePathKey::from_relative_path(Path::new("photo.jpg")).unwrap();
        let mut asset = NewAsset::minimal(
            selection.library_id,
            relative,
            "photo.jpg",
            MediaKind::Jpeg,
            std::fs::metadata(&image_path).unwrap().len(),
        );
        asset.folder_group_id = Some(selection.group_id);
        {
            let mut state = service.state().unwrap();
            state
                .libraries
                .catalog_mut()
                .apply_index_batch(&[
                    CatalogIndexRecord::Discovered(asset.clone()),
                    CatalogIndexRecord::Shaped(AssetShapeUpdate {
                        asset_id: asset.id,
                        width: 8,
                        height: 8,
                        orientation: Some(1),
                        representative_rgb: None,
                        shape_status: ShapeStatus::Ready,
                    }),
                ])
                .unwrap();
            let stored = state
                .libraries
                .catalog()
                .find_asset(asset.id)
                .unwrap()
                .unwrap();
            for class in [DerivativeClass::WallThumbnail]
                .into_iter()
                .chain(with_screen.then_some(DerivativeClass::ScreenPreview))
            {
                let key = DerivativeKey::compute(&super::derivative_spec(&stored, class));
                let kind = super::derivative_kind_name(class);
                state
                    .libraries
                    .catalog_mut()
                    .insert_derivative(&NewDerivative {
                        id: photo_domain::DerivativeId::new(),
                        asset_id: asset.id,
                        folder_group_id: selection.group_id,
                        kind: kind.to_owned(),
                        cache_key: key.as_str().to_owned(),
                        relative_cache_path: std::path::PathBuf::from(format!(
                            "{kind}/{}.bin",
                            asset.id.as_uuid().hyphenated()
                        )),
                        size_bytes: 1,
                        durable: true,
                        created_at: 1,
                    })
                    .unwrap();
            }
        }
        service.coordinator.begin_collection(selection).await;
        let token = service
            .coordinator
            .collection_progress_token(selection)
            .await
            .expect("thumbnail token");
        assert!(service.coordinator.finish_thumbnail_phase(&token).await);
        let derivative_count = Catalog::open(&service.catalog_path)
            .unwrap()
            .all_derivatives()
            .unwrap()
            .len();
        (temp, service, selection, asset.id, derivative_count)
    }

    #[cfg(debug_assertions)]
    async fn assert_stale_collection_gap_is_suppressed(
        with_screen: bool,
        full_group: bool,
        enqueue_gap: bool,
    ) {
        let (_temp, service, selection, asset_id, derivative_count) =
            collection_gap_fixture(with_screen).await;
        let screen_encode_starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        service
            .install_screen_preview_encode_test_counter(screen_encode_starts.clone())
            .await;
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        if enqueue_gap {
            service
                .install_collection_enqueue_test_gate(entered.clone(), release.clone())
                .await;
        } else {
            service
                .install_collection_publish_test_gate(entered.clone(), release.clone())
                .await;
        }
        let mut updates = service.subscribe_wall_updates();
        let generation = service.coordinator.background_generation().await;
        let driver_service = service.clone();
        let recent = if full_group {
            Vec::new()
        } else {
            vec![asset_id]
        };
        let driver = tokio::spawn(async move {
            driver_service
                .run_screen_preview_prefetch_for_generation(
                    recent, full_group, selection, generation,
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("collection driver did not reach the guarded gap");

        service.coordinator.invalidate_background().await;
        service
            .coordinator
            .rewind_collection_to_thumbnails(selection)
            .await;
        release.notify_waiters();
        let outcome = tokio::time::timeout(Duration::from_secs(5), driver)
            .await
            .expect("stale collection driver did not finish")
            .expect("stale collection driver panicked");
        assert!(matches!(outcome, PreviewPrefetchOutcome::Stale));
        assert_eq!(screen_encode_starts.load(Ordering::SeqCst), 0);
        assert_eq!(
            Catalog::open(&service.catalog_path)
                .unwrap()
                .all_derivatives()
                .unwrap()
                .len(),
            derivative_count,
            "stale collection work must not add a screen catalogue row"
        );
        let mut screen_update = false;
        while let Ok(event) = updates.try_recv() {
            if matches!(
                event,
                WallUpdate::DerivativesReady { ref derivatives, .. }
                    if derivatives
                        .iter()
                        .any(|derivative| derivative.kind == DerivativeClass::ScreenPreview)
            ) {
                screen_update = true;
            }
        }
        assert!(
            !screen_update,
            "stale collection work must not publish a screen update"
        );
        assert_eq!(
            service.coordinator.collection_state().await,
            (
                crate::derivative_coordinator::CollectionPhase::Thumbnails,
                None
            )
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "current_thread")]
    async fn collection_driver_is_tracked_before_its_first_poll() {
        let (_temp, service, selection, _asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        service.start_collection_driver(true, selection);
        assert_eq!(
            service.collection_driver_active_count_test(),
            1,
            "driver admission must be visible before the spawned future is first polled"
        );

        let quiescence = service.wait_for_collection_drivers_quiescent_test();
        tokio::pin!(quiescence);
        assert!(
            tokio::time::timeout(Duration::from_millis(0), &mut quiescence)
                .await
                .is_err(),
            "quiescence must wait while the delayed driver is still live"
        );
        tokio::time::timeout(Duration::from_secs(5), quiescence)
            .await
            .expect("collection driver should finish and release its tracking guard");
        assert_eq!(service.collection_driver_active_count_test(), 0);
    }

    #[cfg(debug_assertions)]
    #[test]
    fn collection_driver_keeps_newer_selection_intent_when_older_call_resumes() {
        let control = CollectionDriverControl::new();
        let library_id = LibraryId::from_uuid(uuid::Uuid::from_u128(1));
        let group_id = FolderGroupId::from_uuid(uuid::Uuid::from_u128(2));
        let older = SelectionToken {
            library_id,
            group_id,
            epoch: 7,
        };
        let newer = SelectionToken {
            library_id,
            group_id,
            epoch: 8,
        };

        let admission = control.request(newer, false).unwrap();
        assert!(control.request(newer, true).is_none());
        assert!(control.request(older, false).is_none());
        assert!(control.request(older, true).is_none());

        let intent = match control.take_pending_or_release(admission.owner_id) {
            CollectionDriverTake::Pending(intent) => intent,
            CollectionDriverTake::Released | CollectionDriverTake::Stale => {
                panic!("newer collection intent should remain pending")
            }
        };
        assert_eq!(intent.selection, newer);
        assert!(intent.full_group);
        assert!(control.request(older, true).is_none());
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "current_thread")]
    async fn collection_driver_processes_newer_full_group_after_older_call_resumes() {
        let (_temp, service, older, _asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        let source = _temp.path().join("photos");
        let (_, newer) = service
            .select_recent(&source)
            .expect("same source should produce a newer selection epoch");
        service.coordinator.reset_selection(newer).await;

        let admission = service.collection_driver.request(newer, true).unwrap();
        assert!(service.collection_driver.request(older, true).is_none());
        let _ = service.spawn_collection_driver_task(admission);
        tokio::time::timeout(
            Duration::from_secs(5),
            service.wait_for_collection_drivers_quiescent_test(),
        )
        .await
        .expect("newer collection intent should drain");
        assert_eq!(
            service.coordinator.collection_phase().await,
            crate::derivative_coordinator::CollectionPhase::Complete
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "current_thread")]
    async fn collection_driver_abort_before_first_poll_releases_admission_and_tracking() {
        let (_temp, service, selection, _asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        let admission = service.collection_driver.request(selection, true).unwrap();
        let handle = service
            .spawn_collection_driver_task(admission)
            .expect("collection driver should spawn inside the test runtime");
        assert_eq!(service.collection_driver_active_count_test(), 1);

        handle.abort();
        let _ = handle.await;
        tokio::time::timeout(
            Duration::from_secs(5),
            service.wait_for_collection_drivers_quiescent_test(),
        )
        .await
        .expect("aborted pre-poll driver should release its tracker");
        assert_eq!(service.collection_driver_active_count_test(), 0);

        service.start_collection_driver(true, selection);
        tokio::time::timeout(
            Duration::from_secs(5),
            service.wait_for_collection_drivers_quiescent_test(),
        )
        .await
        .expect("a later request should admit and drain a new driver");
        assert_eq!(service.collection_driver_active_count_test(), 0);
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "current_thread")]
    async fn collection_driver_quiescence_waiter_submits_only_after_pre_poll_cleanup() {
        let (_temp, service, selection, _asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        let admission = service.collection_driver.request(selection, true).unwrap();
        let handle = service
            .spawn_collection_driver_task(admission)
            .expect("collection driver should spawn inside the test runtime");
        let waiter_service = service.clone();
        let waiter = tokio::spawn(async move {
            waiter_service
                .wait_for_collection_drivers_quiescent_test()
                .await;
            assert!(
                !waiter_service.collection_driver_admitted_test(),
                "quiescence must follow owner admission cleanup"
            );
            waiter_service.start_collection_driver(true, selection);
        });
        while service.collection_driver_tracker.waiting_count() == 0 {
            tokio::task::yield_now().await;
        }

        handle.abort();
        let _ = handle.await;
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .expect("quiescence waiter should be released after owner cleanup")
            .expect("quiescence waiter should not panic");
        tokio::time::timeout(
            Duration::from_secs(5),
            service.wait_for_collection_drivers_quiescent_test(),
        )
        .await
        .expect("the one successor should drain");
        assert_eq!(service.collection_driver_active_count_test(), 0);
        assert_eq!(
            service.coordinator.collection_phase().await,
            crate::derivative_coordinator::CollectionPhase::Complete
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "current_thread")]
    async fn collection_driver_cancel_transfers_pending_newer_selection_once() {
        let (_temp, service, older, _asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        let old_admission = service.collection_driver.request(older, true).unwrap();
        let handle = service
            .spawn_collection_driver_task(old_admission)
            .expect("collection driver should spawn inside the test runtime");
        let source = _temp.path().join("photos");
        let (_, newer) = service
            .select_recent(&source)
            .expect("same source should produce a newer selection epoch");
        service.coordinator.reset_selection(newer).await;
        assert!(service.collection_driver.request(newer, true).is_none());

        handle.abort();
        let _ = handle.await;
        tokio::time::timeout(
            Duration::from_secs(5),
            service.wait_for_collection_drivers_quiescent_test(),
        )
        .await
        .expect("newer pending intent should transfer during cancellation");
        assert_eq!(service.collection_driver_active_count_test(), 0);
        assert_eq!(
            service.coordinator.collection_phase().await,
            crate::derivative_coordinator::CollectionPhase::Complete
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "current_thread")]
    async fn collection_driver_panic_transfers_pending_newer_full_group_once() {
        let (_temp, service, older, _asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        let old_admission = service.collection_driver.request(older, false).unwrap();
        let owner = CollectionDriverLifecycleOwner::new(service.clone(), old_admission);
        let source = _temp.path().join("photos");
        let (_, newer) = service
            .select_recent(&source)
            .expect("same source should produce a newer selection epoch");
        service.coordinator.reset_selection(newer).await;
        assert!(service.collection_driver.request(newer, true).is_none());

        let panic_result = catch_unwind(AssertUnwindSafe(|| {
            let _owner = owner;
            panic!("injected collection driver panic");
        }));
        assert!(panic_result.is_err());
        tokio::time::timeout(
            Duration::from_secs(5),
            service.wait_for_collection_drivers_quiescent_test(),
        )
        .await
        .expect("newer full-group intent should transfer during panic cleanup");
        assert_eq!(service.collection_driver_active_count_test(), 0);
        assert_eq!(
            service.coordinator.collection_phase().await,
            crate::derivative_coordinator::CollectionPhase::Complete
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn collection_driver_triggers_coalesce_under_active_interaction() {
        let (_temp, service, _selection, _asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        service
            .set_interaction(crate::InteractionState::Active)
            .await;

        let mut maximum_active = service.collection_driver_active_count_test();
        for index in 0..1_000 {
            if index % 2 == 0 {
                service
                    .set_interaction(crate::InteractionState::Active)
                    .await;
            }
            let recent_id = AssetId::from_uuid(uuid::Uuid::from_u128((index % 500 + 1) as u128));
            service
                .schedule_screen_preview_prefetch(vec![recent_id])
                .await;
            maximum_active = maximum_active.max(service.collection_driver_active_count_test());
            assert!(
                service.coordinator.test_snapshot().await.recent_len <= 250,
                "coalesced intent must retain at most the bounded recent window"
            );
        }
        assert_eq!(
            maximum_active, 1,
            "1,000 active-interaction triggers must admit one collection driver"
        );
        let recent = service.coordinator.recent_ids().await;
        assert_eq!(recent.len(), 250);
        assert_eq!(
            recent.first().copied(),
            Some(AssetId::from_uuid(uuid::Uuid::from_u128(251)))
        );
        assert_eq!(
            recent.last().copied(),
            Some(AssetId::from_uuid(uuid::Uuid::from_u128(500)))
        );

        service.set_interaction(crate::InteractionState::Idle).await;
        tokio::time::timeout(
            Duration::from_secs(5),
            service.wait_for_collection_drivers_quiescent_test(),
        )
        .await
        .expect("coalesced collection intent should drain after interaction ends");
        assert_eq!(service.collection_driver_active_count_test(), 0);
        assert!(service.coordinator.test_snapshot().await.recent_len <= 250);
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn collection_driver_post_take_request_keeps_quiescence_blocked_until_successor_finishes()
    {
        let (_temp, service, _selection, asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        let exit_entered = Arc::new(tokio::sync::Notify::new());
        let exit_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_collection_driver_post_take_test_gate(
                exit_entered.clone(),
                exit_release.clone(),
            )
            .await;

        service
            .schedule_screen_preview_prefetch(vec![asset_id])
            .await;
        tokio::time::timeout(Duration::from_secs(5), exit_entered.notified())
            .await
            .expect("collection driver should reach its post-take exit gate");

        let quiescence = service.wait_for_collection_drivers_quiescent_test();
        tokio::pin!(quiescence);
        assert!(
            tokio::time::timeout(Duration::from_millis(0), &mut quiescence)
                .await
                .is_err()
        );
        service.prefetch_screen_previews(Vec::new()).await;
        assert!(service.collection_driver_admitted_test());
        assert_eq!(
            service.collection_driver_active_count_test(),
            2,
            "post-take request must admit the successor while the old task is tracked"
        );
        let mut maximum_active = service.collection_driver_active_count_test();
        let mut maximum_admitted = usize::from(service.collection_driver_admitted_test());
        for _ in 0..16 {
            tokio::task::yield_now().await;
            service.prefetch_screen_previews(Vec::new()).await;
            maximum_active = maximum_active.max(service.collection_driver_active_count_test());
            maximum_admitted =
                maximum_admitted.max(usize::from(service.collection_driver_admitted_test()));
            assert!(service.collection_driver_admitted_test());
        }
        assert_eq!(maximum_active, 2, "one successor must not be duplicated");
        assert_eq!(maximum_admitted, 1, "only one owner may be admitted");
        assert!(
            tokio::time::timeout(Duration::from_millis(0), &mut quiescence)
                .await
                .is_err(),
            "quiescence must remain blocked while successor work is admitted"
        );

        exit_release.notify_waiters();
        tokio::time::timeout(Duration::from_secs(5), quiescence)
            .await
            .expect("successor should drain after the post-take gate is released");
        assert_eq!(service.collection_driver_active_count_test(), 0);
        assert_eq!(
            service.coordinator.collection_phase().await,
            crate::derivative_coordinator::CollectionPhase::Complete
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn collection_driver_merges_full_group_intent_at_exit_without_lost_wake() {
        let (_temp, service, _selection, asset_id, _derivative_count) =
            collection_gap_fixture(false).await;
        let exit_entered = Arc::new(tokio::sync::Notify::new());
        let exit_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_collection_driver_exit_test_gate(exit_entered.clone(), exit_release.clone())
            .await;

        service
            .schedule_screen_preview_prefetch(vec![asset_id])
            .await;
        tokio::time::timeout(Duration::from_secs(5), exit_entered.notified())
            .await
            .expect("recent collection driver should reach its exit gate");

        service.prefetch_screen_previews(Vec::new()).await;
        exit_release.notify_waiters();
        tokio::time::timeout(
            Duration::from_secs(5),
            service.wait_for_collection_drivers_quiescent_test(),
        )
        .await
        .expect("full-group intent arriving at exit must not be lost");
        assert_eq!(
            service.coordinator.collection_phase().await,
            crate::derivative_coordinator::CollectionPhase::Complete
        );
        assert_eq!(service.collection_driver_active_count_test(), 0);
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stale_recent_cached_preview_publication_is_suppressed_after_invalidation() {
        assert_stale_collection_gap_is_suppressed(true, false, false).await;
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stale_collection_cached_preview_publication_is_suppressed_after_invalidation() {
        assert_stale_collection_gap_is_suppressed(true, true, false).await;
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stale_recent_idle_preview_enqueue_is_suppressed_after_invalidation() {
        assert_stale_collection_gap_is_suppressed(false, false, true).await;
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn stale_collection_idle_preview_enqueue_is_suppressed_after_invalidation() {
        assert_stale_collection_gap_is_suppressed(false, true, true).await;
    }

    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn recent_background_driver_cannot_encode_before_preview_phase() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let image_path = source.join("photo.jpg");
        ImageBuffer::from_pixel(8, 8, Rgb([20_u8, 40, 60]))
            .save(&image_path)
            .unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config).unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        service.coordinator.ensure_selection(selection).await;
        let asset = NewAsset {
            id: AssetId::for_path(
                selection.library_id,
                &RelativePathKey::from_relative_path(Path::new("photo.jpg")).unwrap(),
            ),
            library_id: selection.library_id,
            relative_path: RelativePathKey::from_relative_path(Path::new("photo.jpg")).unwrap(),
            display_path: "photo.jpg".to_owned(),
            media_kind: MediaKind::Jpeg,
            signature: FileSignature {
                size_bytes: std::fs::metadata(&image_path).unwrap().len(),
                modified_unix_ns: 1,
                sidecar_modified_unix_ns: None,
            },
            folder_group_id: Some(selection.group_id),
        };
        {
            let mut state = service.state().unwrap();
            state
                .libraries
                .catalog_mut()
                .apply_index_batch(&[
                    CatalogIndexRecord::Discovered(asset.clone()),
                    CatalogIndexRecord::Shaped(AssetShapeUpdate {
                        asset_id: asset.id,
                        width: 8,
                        height: 8,
                        orientation: Some(1),
                        representative_rgb: None,
                        shape_status: ShapeStatus::Ready,
                    }),
                ])
                .unwrap();
        }
        service.coordinator.begin_collection(selection).await;
        let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Notify::new());
        service
            .install_derivative_test_class_gate_with_counter(
                DerivativeClass::ScreenPreview,
                entered,
                release,
                starts.clone(),
            )
            .await;

        service.run_screen_preview_prefetch(vec![asset.id]).await;

        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            service.coordinator.collection_state().await,
            (
                crate::derivative_coordinator::CollectionPhase::Thumbnails,
                None
            )
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn recent_driver_winning_collection_mutex_cannot_encode_before_full_group() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let image_path = source.join("photo.jpg");
        ImageBuffer::from_pixel(8, 8, Rgb([20_u8, 40, 60]))
            .save(&image_path)
            .unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config).unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        service.coordinator.ensure_selection(selection).await;
        let relative = RelativePathKey::from_relative_path(Path::new("photo.jpg")).unwrap();
        let asset = NewAsset {
            id: AssetId::for_path(selection.library_id, &relative),
            library_id: selection.library_id,
            relative_path: relative,
            display_path: "photo.jpg".to_owned(),
            media_kind: MediaKind::Jpeg,
            signature: FileSignature {
                size_bytes: std::fs::metadata(&image_path).unwrap().len(),
                modified_unix_ns: 1,
                sidecar_modified_unix_ns: None,
            },
            folder_group_id: Some(selection.group_id),
        };
        {
            let mut state = service.state().unwrap();
            state
                .libraries
                .catalog_mut()
                .apply_index_batch(&[
                    CatalogIndexRecord::Discovered(asset.clone()),
                    CatalogIndexRecord::Shaped(AssetShapeUpdate {
                        asset_id: asset.id,
                        width: 8,
                        height: 8,
                        orientation: Some(1),
                        representative_rgb: None,
                        shape_status: ShapeStatus::Ready,
                    }),
                ])
                .unwrap();
        }
        service.coordinator.begin_collection(selection).await;
        let wall_entered = Arc::new(tokio::sync::Notify::new());
        let wall_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_derivative_test_gate(
                asset.id.as_uuid().hyphenated().to_string(),
                DerivativeClass::WallThumbnail,
                wall_entered.clone(),
                wall_release.clone(),
            )
            .await
            .unwrap();
        let screen_starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        service
            .install_screen_preview_encode_test_counter(screen_starts.clone())
            .await;
        service
            .scheduler
            .set_interaction_mode(photo_indexer::InteractionMode::Idle)
            .await;
        // Let the recent intent be admitted first.  Its phase guard must make
        // the recent-only work a no-op, leaving the merged full-group intent
        // free to own the subsequent thumbnail traversal.
        service.coordinator.note_recent(asset.id).await;
        service.start_collection_driver(false, selection);
        tokio::time::sleep(Duration::from_millis(75)).await;
        service.start_collection_driver(true, selection);
        tokio::time::timeout(Duration::from_secs(5), wall_entered.notified())
            .await
            .expect("full-group driver should reach the gated wall");
        assert_eq!(screen_starts.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert_eq!(
            service.coordinator.collection_phase().await,
            crate::derivative_coordinator::CollectionPhase::Thumbnails
        );

        wall_release.notify_waiters();
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if service.coordinator.collection_phase().await
                    == crate::derivative_coordinator::CollectionPhase::Complete
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("full-group driver should finish after the wall release");
        assert!(screen_starts.load(std::sync::atomic::Ordering::SeqCst) > 0);
    }

    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn clearing_one_terminal_class_keeps_warning_for_the_other_class() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let relative = RelativePathKey::from_relative_path(Path::new("photo.jpg")).unwrap();
        let source_path = source.join("photo.jpg");
        ImageBuffer::from_pixel(8, 8, Rgb([20_u8, 40, 60]))
            .save(&source_path)
            .unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config.clone()).unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        let asset = NewAsset {
            id: AssetId::for_path(selection.library_id, &relative),
            library_id: selection.library_id,
            relative_path: relative,
            display_path: "photo.jpg".to_owned(),
            media_kind: MediaKind::Jpeg,
            signature: FileSignature {
                size_bytes: std::fs::metadata(&source_path).unwrap().len(),
                modified_unix_ns: 1,
                sidecar_modified_unix_ns: None,
            },
            folder_group_id: Some(selection.group_id),
        };
        {
            let mut state = service.state().unwrap();
            state
                .libraries
                .catalog_mut()
                .apply_index_batch(&[
                    CatalogIndexRecord::Discovered(asset.clone()),
                    CatalogIndexRecord::Shaped(AssetShapeUpdate {
                        asset_id: asset.id,
                        width: 8,
                        height: 8,
                        orientation: Some(1),
                        representative_rgb: None,
                        shape_status: ShapeStatus::Ready,
                    }),
                ])
                .unwrap();
        }
        let stored_asset = Catalog::open(&config.catalog_path())
            .unwrap()
            .find_asset(asset.id)
            .unwrap()
            .unwrap();
        {
            let mut state = service.state().unwrap();
            for class in [
                DerivativeClass::WallThumbnail,
                DerivativeClass::ScreenPreview,
            ] {
                let key = DerivativeKey::compute(&super::derivative_spec(&stored_asset, class));
                state
                    .libraries
                    .catalog_mut()
                    .record_terminal_derivative_failure(&TerminalDerivativeFailure {
                        asset_id: asset.id,
                        kind: super::derivative_kind_name(class).to_owned(),
                        cache_key: key.as_str().to_owned(),
                        availability: stored_asset.availability,
                        failure_code: "derivative_generation_terminal".to_owned(),
                        occurred_at: 1,
                    })
                    .unwrap();
            }
            state
                .libraries
                .catalog_mut()
                .record_warning_once(&CatalogWarningRecord {
                    library_id: selection.library_id,
                    asset_id: Some(asset.id),
                    code: "derivative_generation_terminal".to_owned(),
                    message: "terminal".to_owned(),
                })
                .unwrap();
        }

        service.clear_derivative_warnings(selection, asset.id, DerivativeClass::WallThumbnail);
        let catalog = Catalog::open(&config.catalog_path()).unwrap();
        let screen_key = DerivativeKey::compute(&super::derivative_spec(
            &stored_asset,
            DerivativeClass::ScreenPreview,
        ));
        assert!(
            catalog
                .find_terminal_derivative_failure(
                    asset.id,
                    "screen_preview",
                    screen_key.as_str(),
                    stored_asset.availability,
                )
                .unwrap()
                .is_some()
        );
        assert_eq!(catalog.warning_count().unwrap(), 1);

        service.clear_derivative_warnings(selection, asset.id, DerivativeClass::ScreenPreview);
        let catalog = Catalog::open(&config.catalog_path()).unwrap();
        assert_eq!(catalog.warning_count().unwrap(), 0);
        assert!(
            catalog
                .find_terminal_derivative_failure(
                    asset.id,
                    "screen_preview",
                    screen_key.as_str(),
                    stored_asset.availability,
                )
                .unwrap()
                .is_none()
        );

        let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
        let wall_key = DerivativeKey::compute(&super::derivative_spec(
            &stored_asset,
            DerivativeClass::WallThumbnail,
        ));
        catalog
            .record_terminal_derivative_failure(&TerminalDerivativeFailure {
                asset_id: asset.id,
                kind: "wall_thumbnail".to_owned(),
                cache_key: wall_key.as_str().to_owned(),
                availability: stored_asset.availability,
                failure_code: "derivative_generation_terminal".to_owned(),
                occurred_at: 2,
            })
            .unwrap();
        catalog
            .record_terminal_derivative_failure(&TerminalDerivativeFailure {
                asset_id: asset.id,
                kind: "screen_preview".to_owned(),
                cache_key: screen_key.as_str().to_owned(),
                availability: stored_asset.availability,
                failure_code: "derivative_generation_terminal".to_owned(),
                occurred_at: 2,
            })
            .unwrap();
        catalog
            .record_warning_once(&CatalogWarningRecord {
                library_id: selection.library_id,
                asset_id: Some(asset.id),
                code: "derivative_generation_terminal".to_owned(),
                message: "terminal".to_owned(),
            })
            .unwrap();
        drop(catalog);

        service.clear_derivative_warnings(selection, asset.id, DerivativeClass::ScreenPreview);
        let catalog = Catalog::open(&config.catalog_path()).unwrap();
        assert!(
            catalog
                .find_terminal_derivative_failure(
                    asset.id,
                    "wall_thumbnail",
                    wall_key.as_str(),
                    stored_asset.availability,
                )
                .unwrap()
                .is_some()
        );
        assert_eq!(catalog.warning_count().unwrap(), 1);
        service.clear_derivative_warnings(selection, asset.id, DerivativeClass::WallThumbnail);
        assert_eq!(
            Catalog::open(&config.catalog_path())
                .unwrap()
                .warning_count()
                .unwrap(),
            0
        );
    }

    #[cfg(debug_assertions)]
    #[tokio::test]
    async fn remaining_idle_group_enumeration_stays_bounded_for_ten_thousand_assets() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let (_bootstrap, selection) = service.select_recent(&source).unwrap();
        service.coordinator.ensure_selection(selection).await;
        {
            let mut state = service.state().unwrap();
            let mut records = Vec::with_capacity(500);
            for index in 0..10_000 {
                let relative = RelativePathKey::from_relative_path(std::path::Path::new(&format!(
                    "photo-{index:05}.jpg"
                )))
                .unwrap();
                let mut asset = NewAsset::minimal(
                    selection.library_id,
                    relative,
                    format!("photo-{index:05}.jpg"),
                    MediaKind::Jpeg,
                    1,
                );
                asset.folder_group_id = Some(selection.group_id);
                records.push(CatalogIndexRecord::Discovered(asset.clone()));
                records.push(CatalogIndexRecord::Shaped(AssetShapeUpdate {
                    asset_id: asset.id,
                    width: 16,
                    height: 9,
                    orientation: Some(1),
                    representative_rgb: None,
                    shape_status: ShapeStatus::Ready,
                }));
                if records.len() >= 500 {
                    state
                        .libraries
                        .catalog_mut()
                        .apply_index_batch(&records)
                        .unwrap();
                    records.clear();
                }
            }
            if !records.is_empty() {
                state
                    .libraries
                    .catalog_mut()
                    .apply_index_batch(&records)
                    .unwrap();
            }
        }
        for index in 0..10_000 {
            let relative = RelativePathKey::from_relative_path(std::path::Path::new(&format!(
                "photo-{index:05}.jpg"
            )))
            .unwrap();
            service
                .coordinator
                .note_recent(AssetId::for_path(selection.library_id, &relative))
                .await;
        }
        let mut cursor = None;
        let mut total = 0;
        loop {
            let (page_selection, page, next) =
                service.remaining_group_ids_page(cursor, None).unwrap();
            assert_eq!(page_selection, selection);
            assert!(page.len() <= 250);
            total += page.len();
            let Some(next) = next else {
                break;
            };
            cursor = Some(next);
        }
        assert_eq!(total, 10_000);
        let snapshot = service.coordinator.test_snapshot().await;
        assert!(snapshot.recent_len <= 250);
        assert!(snapshot.largest_loaded_page <= 250);
        assert!(snapshot.queued_jobs <= 250);
    }

    #[cfg(debug_assertions)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn collection_schedules_bounded_missing_work_with_concurrent_foreground_request() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let (_bootstrap, selection) = service.select_recent(&source).unwrap();
        service.coordinator.ensure_selection(selection).await;
        // Keep fixture construction bounded as well: production collection
        // work must retain only the current page and its cursor, not all
        // catalogue IDs.
        let mut records = Vec::with_capacity(500);
        {
            let mut state = service.state().unwrap();
            for index in 0..10_000 {
                let relative = RelativePathKey::from_relative_path(std::path::Path::new(&format!(
                    "photo-{index:05}.jpg"
                )))
                .unwrap();
                let mut asset = NewAsset::minimal(
                    selection.library_id,
                    relative,
                    format!("photo-{index:05}.jpg"),
                    MediaKind::Jpeg,
                    1,
                );
                asset.folder_group_id = Some(selection.group_id);
                records.push(CatalogIndexRecord::Discovered(asset.clone()));
                records.push(CatalogIndexRecord::Shaped(AssetShapeUpdate {
                    asset_id: asset.id,
                    width: 16,
                    height: 9,
                    orientation: Some(1),
                    representative_rgb: None,
                    shape_status: ShapeStatus::Ready,
                }));
                if records.len() >= 500 {
                    state
                        .libraries
                        .catalog_mut()
                        .apply_index_batch(&records)
                        .unwrap();
                    records.clear();
                }
            }
            if !records.is_empty() {
                state
                    .libraries
                    .catalog_mut()
                    .apply_index_batch(&records)
                    .unwrap();
            }
        }

        let wall_entered = Arc::new(tokio::sync::Notify::new());
        let wall_release = Arc::new(tokio::sync::Notify::new());
        let wall_starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        service
            .install_derivative_test_class_gate_with_counter(
                DerivativeClass::WallThumbnail,
                wall_entered.clone(),
                wall_release.clone(),
                wall_starts.clone(),
            )
            .await;
        service
            .scheduler
            .set_interaction_mode(photo_indexer::InteractionMode::Idle)
            .await;
        let generation = service.coordinator.background_generation().await;
        let collection_service = service.clone();
        let collection = tokio::spawn(async move {
            collection_service
                .run_preview_prefetch_until_stable(Vec::new(), true, selection, generation)
                .await;
        });
        let expected_worker_slots = super::MAX_DERIVATIVE_WORKERS;
        tokio::time::timeout(Duration::from_secs(5), async {
            while wall_starts.load(Ordering::Acquire) < expected_worker_slots {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all derivative worker slots should enter the shared wall gate");
        assert_eq!(
            wall_starts.load(Ordering::Acquire),
            expected_worker_slots,
            "the shared wall gate must hold every active worker slot"
        );

        let catalog_count = {
            let state = service.state().unwrap();
            state
                .libraries
                .catalog()
                .asset_count(selection.library_id)
                .unwrap()
        };
        assert_eq!(catalog_count, 10_000);
        assert!(wall_starts.load(Ordering::SeqCst) >= 1);

        // The collection driver enqueues a complete page before starting its
        // workers.  The class-wide gate holds every wall attempt, so this
        // exact snapshot proves all 250 page jobs are admitted and still
        // queued or running rather than allowing workers to drain the page.
        let collection_snapshot = service.coordinator.test_snapshot().await;
        assert_eq!(collection_snapshot.largest_loaded_page, 250);
        assert!(collection_snapshot.recent_len <= 250);
        assert_eq!(collection_snapshot.queued_jobs, 250);

        let foreground_relative =
            RelativePathKey::from_relative_path(std::path::Path::new("photo-00500.jpg")).unwrap();
        let foreground_id = AssetId::for_path(selection.library_id, &foreground_relative);
        let foreground_entered = Arc::new(tokio::sync::Notify::new());
        let foreground_release = Arc::new(tokio::sync::Notify::new());
        // Replace the page-wide gate only after every collection item is
        // admitted. Existing workers retain the shared gate they observed;
        // the next (foreground) attempt is held independently by asset ID.
        service
            .install_derivative_test_gate(
                foreground_id.as_uuid().hyphenated().to_string(),
                DerivativeClass::WallThumbnail,
                foreground_entered.clone(),
                foreground_release.clone(),
            )
            .await
            .unwrap();
        let foreground_queued = Arc::new(tokio::sync::Notify::new());
        service
            .install_visible_derivative_queue_test_hook(foreground_queued.clone())
            .await;
        let foreground_service = service.clone();
        let foreground = tokio::spawn(async move {
            foreground_service
                .request_derivatives(crate::DerivativeRequest {
                    asset_ids: vec![foreground_id.as_uuid().hyphenated().to_string()],
                    priority: crate::DerivativePriority::NearViewport,
                    kind: DerivativeClass::WallThumbnail,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), foreground_queued.notified())
            .await
            .expect("foreground request should be queued while collection is held");
        let snapshot = service.coordinator.test_snapshot().await;
        assert_eq!(snapshot.largest_loaded_page, 250);
        assert!(snapshot.recent_len <= 250);
        assert_eq!(snapshot.queued_jobs, 251);

        wall_release.notify_waiters();
        tokio::time::timeout(Duration::from_secs(5), foreground_entered.notified())
            .await
            .expect("foreground wall attempt should reach its separate gate");
        foreground_release.notify_waiters();
        // The bound is sampled while both collection and foreground work are
        // live.  Stop those deterministic workers after the observation so a
        // 10k fixture does not turn this bound test into a full image crawl.
        collection.abort();
        foreground.abort();
        let _ = collection.await;
        let _ = foreground.await;
        service.coordinator.abort_all_attempts().await;
        service.coordinator.invalidate_background().await;
        service.wait_for_derivative_tasks_quiescent_test().await;
        assert_eq!(service.derivative_active_task_count_test(), 0);
        assert_eq!(service.coordinator.test_snapshot().await.queued_jobs, 0);
    }

    #[test]
    fn active_interaction_pauses_only_idle_library_derivatives() {
        assert!(super::should_pause(JobPriority::IdleLibrary, 1));
        assert!(super::should_pause(JobPriority::OpenCollection, 1));
        assert!(!super::should_pause(JobPriority::NearViewport, 1));
        assert!(!super::should_pause(JobPriority::Visible, 1));
        assert!(!super::should_pause(JobPriority::IdleLibrary, 4));
    }

    #[test]
    fn stale_screen_preview_commit_cannot_evict_write_or_register() {
        let temp = tempfile::tempdir().unwrap();
        let first = temp.path().join("first");
        let second = temp.path().join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config.clone()).unwrap();
        let (_, stale_selection) = service.select_recent(&first).unwrap();
        service.select_recent(&second).unwrap();
        let asset_id = AssetId::from_uuid(uuid::Uuid::new_v4());
        let spec = DerivativeSpec {
            asset_id,
            signature: photo_domain::FileSignature {
                size_bytes: 10,
                modified_unix_ns: 20,
                sidecar_modified_unix_ns: None,
            },
            orientation: 1,
            kind: DerivativeKind::ScreenPreview,
            decoder_version: "image-0.25-v1".to_owned(),
            colour_space: "srgb".to_owned(),
            target: DerivativeTarget::LongEdge(4096),
        };
        let generator = ImageDerivativeGenerator::new(config.cache_dir()).unwrap();
        let result = service.commit_screen_preview_if_active(
            generator,
            EncodedScreenPreview {
                bytes: vec![1, 2, 3],
                representative_rgb: photo_metadata::RepresentativeRgb {
                    red: 1,
                    green: 2,
                    blue: 3,
                },
            },
            &spec,
            stale_selection.group_id,
            CacheBudget::from_total_space(1024 * 1024),
            &service.protected_groups,
            stale_selection,
            &config.catalog_path(),
        );

        assert!(matches!(
            result,
            Err(DerivativeWorkError::WorkerUnavailable)
        ));
        assert!(
            Catalog::open(&config.catalog_path())
                .unwrap()
                .all_derivatives()
                .unwrap()
                .is_empty()
        );
        assert!(
            !std::fs::read_dir(config.cache_dir()).unwrap().any(|entry| {
                entry
                    .ok()
                    .is_some_and(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            })
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_screen_preview_commits_respect_exact_budget_boundary() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config.clone()).unwrap();
        let (_, selection) = service.select_recent(&source).unwrap();
        let left = AssetId::from_uuid(uuid::Uuid::new_v4());
        let right = AssetId::from_uuid(uuid::Uuid::new_v4());
        {
            let mut state = service.state().unwrap();
            for (asset_id, name) in [(left, "left.jpg"), (right, "right.jpg")] {
                let mut asset = NewAsset::minimal(
                    selection.library_id,
                    RelativePathKey::from_relative_path(std::path::Path::new(name)).unwrap(),
                    name,
                    MediaKind::Jpeg,
                    1,
                );
                asset.id = asset_id;
                asset.folder_group_id = Some(selection.group_id);
                state.libraries.catalog_mut().upsert_asset(&asset).unwrap();
            }
        }
        let make_spec = |asset_id| DerivativeSpec {
            asset_id,
            signature: FileSignature {
                size_bytes: 10,
                modified_unix_ns: 20,
                sidecar_modified_unix_ns: None,
            },
            orientation: 1,
            kind: DerivativeKind::ScreenPreview,
            decoder_version: "image-0.25-v1".to_owned(),
            colour_space: "srgb".to_owned(),
            target: DerivativeTarget::LongEdge(4096),
        };
        let budget = CacheBudget::from_total_space(30);
        let make_commit = |asset_id| {
            let service = service.clone();
            let config = config.clone();
            let spec = make_spec(asset_id);
            tokio::task::spawn_blocking(move || {
                let generator = ImageDerivativeGenerator::new(config.cache_dir()).unwrap();
                service.commit_screen_preview_if_active(
                    generator,
                    EncodedScreenPreview {
                        bytes: vec![1, 2, 3],
                        representative_rgb: photo_metadata::RepresentativeRgb {
                            red: 1,
                            green: 2,
                            blue: 3,
                        },
                    },
                    &spec,
                    selection.group_id,
                    budget,
                    &service.protected_groups,
                    selection,
                    &config.catalog_path(),
                )
            })
        };
        let (left_result, right_result) = tokio::join!(make_commit(left), make_commit(right));
        let outcomes = [left_result.unwrap(), right_result.unwrap()];
        assert_eq!(
            outcomes.iter().filter(|result| result.is_ok()).count(),
            1,
            "screen-preview commit outcomes: {outcomes:#?}"
        );
        assert_eq!(outcomes.iter().filter(|result| result.is_err()).count(), 1);
        assert!(outcomes.iter().any(|result| {
            matches!(
                result,
                Err(DerivativeWorkError::Image(
                    photo_cache::ImageDerivativeError::Cache(
                        photo_cache::CacheError::BudgetExceeded
                    )
                ))
            )
        }));
        assert_eq!(
            Catalog::open(&config.catalog_path())
                .unwrap()
                .non_durable_size_bytes()
                .unwrap(),
            3
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn cache_wide_failures_are_coalesced_and_a_successful_retry_clears_the_warning() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        for index in 0..250 {
            ImageBuffer::from_pixel(2, 2, Rgb([index as u8, 20, 30]))
                .save(source.join(format!("photo-{index:03}.jpg")))
                .unwrap();
        }
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let mut service = AppService::open(config.clone()).unwrap();
        service.cache_budget = CacheBudget::from_total_space(0);
        let mut updates = service.subscribe_wall_updates();
        service.start_scan(&source).await.unwrap();
        receive_until(&mut updates, |event| {
            matches!(event, WallUpdate::MetadataSettled { .. })
        })
        .await;
        let ids = service
            .query_wall(crate::WallQueryRequest {
                cursor: None,
                limit: 250,
                direction: crate::SortDirection::OldestFirst,
            })
            .await
            .unwrap()
            .items
            .into_iter()
            .map(|asset| asset.id)
            .collect::<Vec<_>>();
        service
            .run_screen_preview_prefetch(
                ids.iter()
                    .map(|id| AssetId::from_uuid(uuid::Uuid::parse_str(id).unwrap()))
                    .collect(),
            )
            .await;
        let warning = receive_until(&mut updates, |event| {
            matches!(event, WallUpdate::Warning { .. })
        })
        .await;
        assert!(matches!(
            warning,
            WallUpdate::Warning { asset_id: None, .. }
        ));
        let warning_json = serde_json::to_string(&warning).unwrap();
        assert!(warning_json.len() < 512);
        assert!(!warning_json.contains(&source.to_string_lossy().into_owned()));
        let mut cache_warning_updates = 1;
        while let Ok(update) = updates.try_recv() {
            if matches!(
                update,
                WallUpdate::Warning {
                    asset_id: None,
                    warning: crate::WallWarningState { ref code, .. },
                    ..
                } if code == "screenPreviewCacheUnavailable"
            ) {
                cache_warning_updates += 1;
            }
        }
        assert_eq!(cache_warning_updates, 1);
        assert_eq!(
            photo_catalog::Catalog::open(&config.catalog_path())
                .unwrap()
                .warning_count()
                .unwrap(),
            1,
            "250 cache failures must produce one library warning"
        );
        let source_snapshot = service
            .query_wall(crate::WallQueryRequest::oldest_first())
            .await
            .unwrap();
        assert_eq!(
            source_snapshot
                .source_warnings
                .iter()
                .map(|warning| warning.code.as_str())
                .collect::<Vec<_>>(),
            vec!["screenPreviewCacheUnavailable"]
        );

        drop(service);
        let reopen_config = config.clone();
        let mut service = std::thread::spawn(move || AppService::open(reopen_config))
            .join()
            .unwrap()
            .unwrap();
        service.cache_budget = CacheBudget::from_total_space(0);
        let mut updates = service.subscribe_wall_updates();
        let reopened_snapshot = service
            .query_wall(crate::WallQueryRequest::oldest_first())
            .await
            .unwrap();
        assert_eq!(
            reopened_snapshot
                .source_warnings
                .iter()
                .map(|warning| warning.code.as_str())
                .collect::<Vec<_>>(),
            vec!["screenPreviewCacheUnavailable"]
        );
        let (_, ready_after_reopen, pending_after_reopen) = service
            .resolve_derivatives(
                &ids.iter()
                    .map(|id| AssetId::from_uuid(uuid::Uuid::parse_str(id).unwrap()))
                    .collect::<Vec<_>>(),
                DerivativeClass::ScreenPreview,
            )
            .unwrap();
        assert_eq!(ready_after_reopen.len(), 0);
        assert_eq!(pending_after_reopen.len(), 250);
        service
            .run_derivative_jobs(pending_after_reopen, JobPriority::NearViewport)
            .await;
        let repeated = receive_until(&mut updates, |event| {
            matches!(event, WallUpdate::Warning { asset_id: None, .. })
        })
        .await;
        assert!(matches!(
            repeated,
            WallUpdate::Warning {
                asset_id: None,
                warning: crate::WallWarningState { ref code, .. },
                ..
            } if code == "screenPreviewCacheUnavailable"
        ));

        let (_, cached_wall, pending_wall) = service
            .resolve_derivatives(
                &ids.iter()
                    .map(|id| AssetId::from_uuid(uuid::Uuid::parse_str(id).unwrap()))
                    .collect::<Vec<_>>(),
                DerivativeClass::WallThumbnail,
            )
            .unwrap();
        assert_eq!(cached_wall.len(), 250);
        assert!(pending_wall.is_empty());
        assert_eq!(
            service
                .run_derivative_jobs(pending_wall, JobPriority::Visible)
                .await
                .len(),
            0
        );
        assert_eq!(
            photo_catalog::Catalog::open(&config.catalog_path())
                .unwrap()
                .warning_count()
                .unwrap(),
            1,
            "wall success must not clear an active screen-cache warning"
        );
        assert!(
            updates.try_recv().is_err(),
            "wall success must not publish a clear for a screen-cache warning"
        );

        service.cache_budget = CacheBudget::from_total_space(10_000_000);
        let (_, cached_after_recovery, pending_after_recovery) = service
            .resolve_derivatives(
                &ids.iter()
                    .map(|id| AssetId::from_uuid(uuid::Uuid::parse_str(id).unwrap()))
                    .collect::<Vec<_>>(),
                DerivativeClass::ScreenPreview,
            )
            .unwrap();
        assert!(cached_after_recovery.is_empty());
        let recovered = service
            .run_derivative_jobs(pending_after_recovery, JobPriority::NearViewport)
            .await;
        assert_eq!(recovered.len(), 250);
        let cleared = receive_until(&mut updates, |event| {
            matches!(
                event,
                WallUpdate::WarningCleared {
                    asset_id: None,
                    code,
                    ..
                } if code == "screenPreviewCacheUnavailable"
            )
        })
        .await;
        let cleared_json = serde_json::to_string(&cleared).unwrap();
        assert!(cleared_json.len() < 256);
        assert!(!cleared_json.contains(&source.to_string_lossy().into_owned()));
        let mut screen_clear_updates = 1;
        while let Ok(update) = updates.try_recv() {
            if matches!(
                update,
                WallUpdate::WarningCleared {
                    asset_id: None,
                    ref code,
                    ..
                } if code == "screenPreviewCacheUnavailable"
            ) {
                screen_clear_updates += 1;
            }
        }
        assert_eq!(screen_clear_updates, 1);
        assert_eq!(
            photo_catalog::Catalog::open(&config.catalog_path())
                .unwrap()
                .warning_count()
                .unwrap(),
            0,
            "a successful cache write must clear the recovered library warning"
        );
    }

    async fn receive_until(
        receiver: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
        mut predicate: impl FnMut(&WallUpdate) -> bool,
    ) -> WallUpdate {
        let mut last = None;
        loop {
            match tokio::time::timeout(std::time::Duration::from_secs(15), receiver.recv()).await {
                Ok(Ok(event)) if predicate(&event) => return event,
                Ok(Ok(event)) => last = Some(event),
                Ok(Err(error)) => panic!("update receiver failed after {last:?}: {error}"),
                Err(_) => panic!(
                    "timed out waiting for update; received a prior event: {}",
                    last.is_some()
                ),
            }
        }
    }
}
