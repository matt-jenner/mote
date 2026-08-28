//! Derivative request orchestration lives here so source paths never cross the DTO boundary.

use std::collections::HashSet;
use std::path::PathBuf;
#[cfg(debug_assertions)]
use std::sync::Arc;
#[cfg(debug_assertions)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use photo_cache::{
    DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget, ImageDerivativeError,
    ImageDerivativeGenerator,
};
use photo_catalog::{Catalog, NewDerivative, WallOrder};
use photo_domain::{AssetId, Availability, DerivativeId, FolderGroupId, MediaKind};
use photo_indexer::{InteractionMode, JobPriority};

use crate::derivative_coordinator::{CommitPermit, WorkKey, WorkTicket};
use crate::service::SelectionToken;
use crate::{
    AppService, AppServiceError, DerivativeClass, DerivativePriority, DerivativeReference,
    DerivativeRequest, WallUpdate,
};

const DECODER_VERSION: &str = "image-0.25-v1";
const ASSET_DERIVATIVE_WARNING: &str = "derivative_generation_failed";
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

struct GeneratedWork {
    reference: DerivativeReference,
    permit: CommitPermit,
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
            | Self::Image(ImageDerivativeError::Decode(_))
            | Self::Image(ImageDerivativeError::UnsupportedTarget)
            | Self::Image(ImageDerivativeError::BudgetAuthorizationRequired)
            | Self::Image(ImageDerivativeError::InvalidSpecification) => {
                DerivativeFailureScope::Asset
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
    spec: DerivativeSpec,
    group: FolderGroupId,
    class: DerivativeClass,
    selection: SelectionToken,
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
            });
            return Ok(());
        }

        let request_selection = self.active_selection_token()?;
        let ids = {
            let state = self.state()?;
            validate_requested_assets(state.libraries.catalog(), request_selection.group_id, &ids)?
        };
        if ids.is_empty() {
            return Ok(());
        }
        self.coordinator.ensure_selection(request_selection).await;
        if request.priority == DerivativePriority::Visible
            && request.kind == DerivativeClass::WallThumbnail
        {
            self.coordinator.invalidate_background().await;
            #[cfg(debug_assertions)]
            if let Some(hook) = self.derivative_request_test_hook.lock().await.clone() {
                hook.started.notify_one();
                if let Some(release) = hook.release {
                    release.notified().await;
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
            for id in successful_in_request.iter().copied() {
                self.coordinator.note_recent(id).await;
            }
            self.schedule_screen_preview_prefetch(successful_in_request)
                .await;
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
                        spec,
                        group,
                        class,
                        selection: selection_token,
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
        self.run_derivative_jobs_with_publication(pending, lane)
            .await
    }

    async fn run_derivative_jobs_publishing(
        &self,
        pending: Vec<PendingDerivative>,
        lane: crate::derivative_coordinator::WorkLane,
    ) -> Vec<DerivativeReference> {
        self.run_derivative_jobs_with_publication(pending, lane)
            .await
    }

    async fn run_derivative_jobs_with_publication(
        &self,
        pending: Vec<PendingDerivative>,
        lane: crate::derivative_coordinator::WorkLane,
    ) -> Vec<DerivativeReference> {
        let mut receivers = Vec::with_capacity(pending.len());
        for pending in pending {
            let key = WorkKey {
                selection: pending.selection,
                asset_id: pending.id,
                class: pending.class,
                cache_key: DerivativeKey::compute(&pending.spec).as_str().to_owned(),
            };
            let prerequisite = (pending.class == DerivativeClass::ScreenPreview).then(|| {
                DerivativeKey::compute(&wall_spec_from(&pending.spec))
                    .as_str()
                    .to_owned()
            });
            receivers.push(
                self.coordinator
                    .enqueue_with_prerequisite(key, lane, prerequisite)
                    .await,
            );
        }
        if !receivers.is_empty() {
            #[cfg(debug_assertions)]
            if lane == crate::derivative_coordinator::WorkLane::VisibleWall
                && let Some(hook) = self.derivative_visible_queue_test_hook.lock().await.clone()
            {
                hook.notify_one();
            }
            self.start_derivative_driver();
        }
        let mut ready = Vec::new();
        for receiver in receivers {
            if let Ok(Some(reference)) = receiver.await {
                ready.push(reference);
            }
        }
        ready
    }

    fn start_derivative_driver(&self) {
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
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
            let _ = worker.await;
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
            let selection = key.selection;
            let asset_id = key.asset_id;
            let class = key.class;
            let outcome = if self.selection_is_active(selection) {
                self.generate_derivative(ticket, key).await
            } else {
                Err(GenerationFailure {
                    error: DerivativeWorkError::WorkerUnavailable,
                    permit: None,
                })
            };
            match outcome {
                Ok(GeneratedWork { reference, permit }) => {
                    self.clear_derivative_warnings(selection, asset_id, class);
                    self.publish_derivatives_if_active(selection, vec![reference.clone()]);
                    self.coordinator.complete_commit(permit, reference).await;
                }
                Err(GenerationFailure { error, permit }) => {
                    if !matches!(error, DerivativeWorkError::PreviewSuperseded)
                        && !matches!(error, DerivativeWorkError::WallThumbnailRequired)
                    {
                        match error.scope() {
                            DerivativeFailureScope::Asset => {
                                self.record_asset_derivative_warning(selection, asset_id);
                            }
                            DerivativeFailureScope::CacheWide => {
                                self.record_cache_derivative_warning(selection, class);
                            }
                        }
                    }
                    if let Some(permit) = permit {
                        self.coordinator.fail_commit(permit).await;
                    } else {
                        self.coordinator.discard(ticket).await;
                    }
                }
            }
        }
    }

    async fn generate_derivative(
        &self,
        ticket: WorkTicket,
        key: WorkKey,
    ) -> Result<GeneratedWork, GenerationFailure> {
        let pending = self
            .pending_for_key(&key)
            .map_err(|error| GenerationFailure {
                error,
                permit: None,
            })?
            .ok_or_else(|| GenerationFailure {
                error: DerivativeWorkError::WorkerUnavailable,
                permit: None,
            })?;
        let id = pending.id;
        let group = pending.group;
        let class = pending.class;
        let selection = pending.selection;
        #[cfg(any(test, debug_assertions))]
        self.wait_for_test_derivative_gate(id, class).await;
        if !self.selection_is_active(selection) {
            return Err(GenerationFailure {
                error: DerivativeWorkError::WorkerUnavailable,
                permit: None,
            });
        }
        let generator =
            ImageDerivativeGenerator::new(&self.cache_root).map_err(|error| GenerationFailure {
                error: error.into(),
                permit: None,
            })?;
        match class {
            DerivativeClass::WallThumbnail => {
                let permit = self
                    .coordinator
                    .admit_commit(ticket, selection, None)
                    .await
                    .ok_or_else(|| GenerationFailure {
                        error: DerivativeWorkError::PreviewSuperseded,
                        permit: None,
                    })?;
                let source = pending.source;
                let cached_source = pending.cached_source.clone().or(self
                    .cached_screen_preview_for_wall(id, group, &pending.spec)
                    .map_err(|error| GenerationFailure {
                        error,
                        permit: Some(permit),
                    })?);
                let spec = pending.spec;
                let generated = tokio::task::spawn_blocking(move || {
                    if let Some(cached_source) = cached_source {
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
                })
                .await
                .map_err(|_| GenerationFailure {
                    error: DerivativeWorkError::WorkerUnavailable,
                    permit: Some(permit),
                })?
                .map_err(|error| GenerationFailure {
                    error: error.into(),
                    permit: Some(permit),
                })?;
                let mut state = self.state().map_err(|_| GenerationFailure {
                    error: DerivativeWorkError::WorkerUnavailable,
                    permit: Some(permit),
                })?;
                let catalog_started = std::time::Instant::now();
                let catalog_result =
                    state
                        .libraries
                        .catalog_mut()
                        .upsert_derivative(&NewDerivative {
                            id: DerivativeId::new(),
                            asset_id: id,
                            folder_group_id: group,
                            kind: derivative_kind_name(class).into(),
                            cache_key: generated.key.as_str().into(),
                            relative_cache_path: generated.relative_path,
                            size_bytes: generated.size_bytes,
                            durable: true,
                            created_at: 0,
                        });
                record_timing_stage("catalog_commit", catalog_started);
                catalog_result.map_err(|error| GenerationFailure {
                    error: ImageDerivativeError::Catalog(error).into(),
                    permit: Some(permit),
                })?;
                Ok(GeneratedWork {
                    reference: reference(id, class, generated.key.as_str().into()),
                    permit,
                })
            }
            DerivativeClass::ScreenPreview => {
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
                let encoded = tokio::task::spawn_blocking({
                    let generator = generator.clone();
                    let spec = spec.clone();
                    move || generator.encode_screen_preview(&source, &spec)
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
                let cache_budget = self.cache_budget;
                let protected = self.protected_groups.clone();
                let cache_root = self.cache_root.clone();
                let catalog_path = self.catalog_path.clone();
                #[cfg(any(test, debug_assertions))]
                let commit_counter = self.screen_preview_commit_test_counter.lock().await.clone();
                let generated = tokio::task::spawn_blocking(move || {
                    let generator = ImageDerivativeGenerator::new(&cache_root)?;
                    let mut catalog = Catalog::open(&catalog_path)?;
                    #[cfg(any(test, debug_assertions))]
                    if let Some(counter) = commit_counter {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                    generator.commit_screen_preview(
                        encoded,
                        &spec,
                        group,
                        &mut catalog,
                        cache_budget,
                        &protected,
                    )
                })
                .await
                .map_err(|_| GenerationFailure {
                    error: DerivativeWorkError::WorkerUnavailable,
                    permit: Some(permit),
                })?
                .map_err(|error| GenerationFailure {
                    error: error.into(),
                    permit: Some(permit),
                })?;
                Ok(GeneratedWork {
                    reference: reference(id, class, generated.key.as_str().into()),
                    permit,
                })
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
            spec,
            group,
            class: key.class,
            selection: key.selection,
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
            if let Some(starts) = gate.starts {
                starts.fetch_add(1, Ordering::SeqCst);
            }
            if gate.blocked_asset.is_none() || gate.blocked_asset == Some(asset_id) {
                gate.entered.notify_one();
                gate.release.notified().await;
            }
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
            gate.entered.notify_one();
            gate.release.notified().await;
        }
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

    /// Installs a deterministic counter at the managed screen-preview commit call.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub async fn install_screen_preview_commit_test_counter(&self, counter: Arc<AtomicUsize>) {
        *self.screen_preview_commit_test_counter.lock().await = Some(counter);
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
        let generation = self.coordinator.background_generation().await;
        if tokio::runtime::Handle::try_current().is_err() {
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            service
                .run_preview_prefetch_until_stable(recent, full_group, selection, generation)
                .await;
        });
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
                    self.coordinator
                        .wait_for_change_since(observed_change)
                        .await;
                    observed_change = self.coordinator.change_generation();
                }
                PreviewPrefetchOutcome::Complete | PreviewPrefetchOutcome::Terminate => return,
                PreviewPrefetchOutcome::Stale => {
                    if !self.selection_is_active(selection) {
                        return;
                    }
                    generation = self.coordinator.background_generation().await;
                    observed_change = self.coordinator.change_generation();
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
        self.run_preview_prefetch_until_stable(recent, true, selection, generation)
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
        let recent_ids = recent.clone();
        let recent = recent.into_iter().collect::<HashSet<_>>();
        let Ok((wall_selection, wall_ready)) = self
            .ensure_wall_thumbnails(&recent_ids, DerivativePriority::NearViewport)
            .await
        else {
            return PreviewPrefetchOutcome::Terminate;
        };
        if wall_selection != selection
            || self.coordinator.background_generation().await != generation
        {
            return PreviewPrefetchOutcome::Stale;
        }
        if wall_ready.len() != recent.len() {
            return PreviewPrefetchOutcome::Blocked;
        }
        if !full_group {
            let recent_ready = recent.iter().copied().collect::<Vec<_>>();
            let Ok((screen_selection, cached, pending)) =
                self.resolve_screen_previews(&recent_ready)
            else {
                return PreviewPrefetchOutcome::Terminate;
            };
            if screen_selection != selection {
                return PreviewPrefetchOutcome::Stale;
            }
            if !cached.is_empty() {
                self.publish_derivatives_if_active(screen_selection, cached);
            }
            let _generated = self
                .run_derivative_jobs_publishing(
                    pending,
                    crate::derivative_coordinator::WorkLane::IdlePreview,
                )
                .await;
            return PreviewPrefetchOutcome::Complete;
        }

        self.coordinator.begin_collection(selection).await;
        let mut cursor = None;
        loop {
            if self.coordinator.background_generation().await != generation {
                return PreviewPrefetchOutcome::Stale;
            }
            if !self.preview_work_can_continue(selection).await {
                return PreviewPrefetchOutcome::Blocked;
            }
            let Ok((page_selection, remaining, next)) =
                self.remaining_group_ids_page(&recent, cursor)
            else {
                return PreviewPrefetchOutcome::Terminate;
            };
            if page_selection != selection {
                return PreviewPrefetchOutcome::Stale;
            }
            if !remaining.is_empty() {
                let Ok((wall_selection, wall_ready)) = self
                    .ensure_wall_thumbnails_lane(
                        &remaining,
                        crate::derivative_coordinator::WorkLane::IdleWall,
                    )
                    .await
                else {
                    return PreviewPrefetchOutcome::Terminate;
                };
                if wall_selection != selection || wall_ready.len() != remaining.len() {
                    return PreviewPrefetchOutcome::Blocked;
                }
            }
            self.coordinator
                .advance_collection(selection, next.clone())
                .await;
            let Some(next) = next else {
                break;
            };
            cursor = Some(next);
        }
        self.coordinator.finish_thumbnail_phase(selection).await;

        let recent_ready = recent.iter().copied().collect::<Vec<_>>();
        let Ok((screen_selection, cached, pending)) = self.resolve_screen_previews(&recent_ready)
        else {
            return PreviewPrefetchOutcome::Terminate;
        };
        if screen_selection != selection {
            return PreviewPrefetchOutcome::Stale;
        }
        if !cached.is_empty() {
            self.publish_derivatives_if_active(screen_selection, cached);
        }
        let _generated = self
            .run_derivative_jobs_publishing(
                pending,
                crate::derivative_coordinator::WorkLane::IdlePreview,
            )
            .await;

        let mut cursor = None;
        loop {
            if self.coordinator.background_generation().await != generation {
                return PreviewPrefetchOutcome::Stale;
            }
            if !self.preview_work_can_continue(selection).await {
                return PreviewPrefetchOutcome::Blocked;
            }
            let Ok((page_selection, remaining, next)) =
                self.remaining_group_ids_page(&recent, cursor)
            else {
                return PreviewPrefetchOutcome::Terminate;
            };
            if page_selection != selection {
                return PreviewPrefetchOutcome::Stale;
            }
            if !remaining.is_empty() {
                let Ok((resolved_selection, cached, pending)) =
                    self.resolve_screen_previews(&remaining)
                else {
                    return PreviewPrefetchOutcome::Terminate;
                };
                if resolved_selection != selection {
                    return PreviewPrefetchOutcome::Stale;
                }
                if !cached.is_empty() {
                    self.publish_derivatives_if_active(resolved_selection, cached);
                }
                let _generated = self
                    .run_derivative_jobs_publishing(
                        pending,
                        crate::derivative_coordinator::WorkLane::IdlePreview,
                    )
                    .await;
            }
            self.coordinator
                .advance_collection(selection, next.clone())
                .await;
            let Some(next) = next else {
                break;
            };
            cursor = Some(next);
        }
        self.coordinator.finish_preview_phase(selection).await;
        PreviewPrefetchOutcome::Complete
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

    async fn ensure_wall_thumbnails_lane(
        &self,
        ids: &[AssetId],
        lane: crate::derivative_coordinator::WorkLane,
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
        let generated = self.run_derivative_jobs_publishing(pending, lane).await;
        ready_ids.extend(
            generated
                .iter()
                .filter_map(|reference| uuid::Uuid::parse_str(&reference.asset_id).ok())
                .map(AssetId::from_uuid),
        );
        Ok((selection, ready_ids))
    }

    fn remaining_group_ids_page(
        &self,
        recent: &HashSet<AssetId>,
        cursor: Option<photo_catalog::WallCursorKey>,
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
        let page = state
            .libraries
            .catalog()
            .wall_page(group, order, cursor, 250)?;
        Ok((
            SelectionToken {
                library_id: selection.library_id,
                group_id: group,
                epoch: state.selection_epoch,
            },
            page.items
                .into_iter()
                .map(|asset| asset.id)
                .filter(|id| !recent.contains(id))
                .collect(),
            page.next,
        ))
    }

    fn active_selection_token(&self) -> Result<SelectionToken, AppServiceError> {
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
        let publication_started = std::time::Instant::now();
        let result = self.updates.send(WallUpdate::DerivativesReady {
            selection_id: selection.selection_id(),
            derivatives,
        });
        record_timing_stage("derivative_publication", publication_started);
        result.is_ok()
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
            state.libraries.catalog_mut().record_warning_once(
                &photo_catalog::CatalogWarningRecord {
                    library_id: selection.library_id,
                    asset_id: None,
                    code: catalog_code.to_owned(),
                    message: message.to_owned(),
                },
            )?;
            let published = match class {
                DerivativeClass::WallThumbnail => &mut state.published_wall_cache_warning,
                DerivativeClass::ScreenPreview => &mut state.published_screen_cache_warning,
            };
            if *published != Some(selection)
                && self
                    .updates
                    .send(WallUpdate::Warning {
                        selection_id: selection.selection_id(),
                        source_id: selection.library_id.as_uuid().hyphenated().to_string(),
                        asset_id: None,
                        warning,
                    })
                    .is_ok()
            {
                *published = Some(selection);
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
            state
                .libraries
                .catalog_mut()
                .clear_terminal_derivative_failure(asset_id, derivative_kind_name(class))?;
            let source_id = selection.library_id.as_uuid().hyphenated().to_string();
            if asset_warning_removed > 0 {
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
    ids: &[AssetId],
) -> Result<Vec<AssetId>, AppServiceError> {
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
    priority == JobPriority::IdleLibrary && background_permits <= 1
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
    use std::path::Path;
    use std::sync::{Arc, Condvar, Mutex};
    use std::time::Duration;

    use image::{ImageBuffer, Rgb};
    use photo_cache::{
        CacheBudget, DerivativeKind, DerivativeSpec, DerivativeTarget, EncodedScreenPreview,
        ImageDerivativeGenerator,
    };
    use photo_catalog::{AssetShapeUpdate, Catalog, CatalogIndexRecord, NewAsset, ShapeStatus};
    use photo_domain::{AssetId, FileSignature, MediaKind, RelativePathKey};
    use photo_indexer::{JobPriority, MetadataReader};
    use photo_metadata::{MetadataBundle, MetadataReadWarning};

    use crate::service::DerivativeTestGate;
    use crate::{AppConfig, AppService, DerivativeClass, WallUpdate};

    use super::DerivativeWorkError;

    struct BlockingReader {
        gate: Arc<(Mutex<bool>, Condvar)>,
        entered: Arc<tokio::sync::Notify>,
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
        let prefetch_service = service.clone();
        let prefetch = tokio::spawn(async move {
            prefetch_service
                .run_preview_prefetch_until_stable(Vec::new(), true, selection, generation)
                .await;
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(
            !prefetch.is_finished(),
            "prefetch did not block while the scan was active"
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
        assert!(
            service.coordinator.change_generation() > change_generation,
            "scan completion did not record its explicit coordinator wake"
        );
        tokio::time::timeout(Duration::from_secs(5), prefetch)
            .await
            .expect("prefetch was not woken by scan completion")
            .expect("prefetch task panicked");
    }

    #[test]
    fn remaining_idle_group_enumeration_paginates_past_two_hundred_and_fifty_assets() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let (_bootstrap, selection) = service.select_recent(&source).unwrap();
        {
            let mut state = service.state().unwrap();
            let mut shapes = Vec::new();
            for index in 0..251 {
                let relative = RelativePathKey::from_relative_path(std::path::Path::new(&format!(
                    "photo-{index:03}.jpg"
                )))
                .unwrap();
                let mut asset = NewAsset::minimal(
                    selection.library_id,
                    relative,
                    format!("photo-{index:03}.jpg"),
                    MediaKind::Jpeg,
                    1,
                );
                asset.folder_group_id = Some(selection.group_id);
                state.libraries.catalog_mut().upsert_asset(&asset).unwrap();
                shapes.push(CatalogIndexRecord::Shaped(AssetShapeUpdate {
                    asset_id: asset.id,
                    width: 16,
                    height: 9,
                    orientation: Some(1),
                    representative_rgb: None,
                    shape_status: ShapeStatus::Ready,
                }));
            }
            state
                .libraries
                .catalog_mut()
                .apply_index_batch(&shapes)
                .unwrap();
        }
        let recent = std::collections::HashSet::new();

        let (first_selection, first, first_cursor) =
            service.remaining_group_ids_page(&recent, None).unwrap();
        let (second_selection, second, second_cursor) = service
            .remaining_group_ids_page(&recent, first_cursor)
            .unwrap();
        let (_, tail, tail_cursor) = service
            .remaining_group_ids_page(&recent, second_cursor)
            .unwrap();

        assert_eq!(first_selection, selection);
        assert_eq!(second_selection, selection);
        assert_eq!(first.len(), 250);
        assert_eq!(second.len(), 1);
        assert!(tail.is_empty());
        assert!(tail_cursor.is_none());
    }

    #[test]
    fn active_interaction_pauses_only_idle_library_derivatives() {
        assert!(super::should_pause(JobPriority::IdleLibrary, 1));
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
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
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
