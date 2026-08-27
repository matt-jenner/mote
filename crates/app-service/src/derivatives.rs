//! Derivative request orchestration lives here so source paths never cross the DTO boundary.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
#[cfg(debug_assertions)]
use std::sync::Arc;
#[cfg(debug_assertions)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use photo_cache::{
    DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget, ImageDerivativeError,
    ImageDerivativeGenerator,
};
use photo_catalog::{Catalog, NewDerivative, WallOrder};
use photo_domain::{AssetId, Availability, DerivativeId, FolderGroupId};
use photo_indexer::{IndexJob, InteractionMode, JobPriority};
use tokio::sync::{Mutex, oneshot};

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
const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(50);

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
    preview_generation: Option<u64>,
}

struct SharedDerivative {
    pending: PendingDerivative,
    job_name: String,
    waiters: Vec<oneshot::Sender<Option<DerivativeReference>>>,
}

#[derive(Default)]
struct QueueState {
    by_key: HashMap<String, SharedDerivative>,
    job_to_key: HashMap<String, String>,
}

#[derive(Default)]
pub(crate) struct DerivativeQueue {
    state: Mutex<QueueState>,
    driver: Mutex<()>,
    sequence: AtomicU64,
}

impl DerivativeQueue {
    async fn enqueue(
        &self,
        scheduler: &photo_indexer::IndexScheduler,
        pending: PendingDerivative,
        priority: JobPriority,
    ) -> oneshot::Receiver<Option<DerivativeReference>> {
        let immutable_key = DerivativeKey::compute(&pending.spec).as_str().to_owned();
        let (send, receive) = oneshot::channel();
        let mut state = self.state.lock().await;
        if let Some(existing) = state.by_key.get_mut(&immutable_key)
            && existing.pending.selection == pending.selection
        {
            existing.waiters.push(send);
            let job_name = existing.job_name.clone();
            drop(state);
            scheduler.enqueue(IndexJob::new(job_name, priority)).await;
            return receive;
        }
        if let Some(replaced) = state.by_key.remove(&immutable_key) {
            state.job_to_key.remove(&replaced.job_name);
        }
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let job_name = format!("photo-derivative:{sequence}");
        state
            .job_to_key
            .insert(job_name.clone(), immutable_key.clone());
        state.by_key.insert(
            immutable_key,
            SharedDerivative {
                pending,
                job_name: job_name.clone(),
                waiters: vec![send],
            },
        );
        drop(state);
        scheduler.enqueue(IndexJob::new(job_name, priority)).await;
        receive
    }

    async fn next_owned(
        &self,
        scheduler: &photo_indexer::IndexScheduler,
    ) -> Option<(IndexJob, String, PendingDerivative)> {
        let mut foreign = Vec::new();
        while let Some(job) = scheduler.next().await {
            let owned = {
                let mut state = self.state.lock().await;
                state.job_to_key.remove(job.name()).and_then(|key| {
                    state
                        .by_key
                        .get(&key)
                        .map(|work| (key, work.pending.clone()))
                })
            };
            if let Some((key, pending)) = owned {
                for job in foreign {
                    scheduler.enqueue(job).await;
                }
                return Some((job, key, pending));
            }
            if job.name().starts_with("photo-derivative:") {
                continue;
            }
            foreign.push(job);
        }
        for job in foreign {
            scheduler.enqueue(job).await;
        }
        None
    }

    async fn requeue(&self, job: &IndexJob, key: &str) {
        let mut state = self.state.lock().await;
        if state.by_key.contains_key(key) {
            state
                .job_to_key
                .insert(job.name().to_owned(), key.to_owned());
        }
    }

    async fn finish(&self, key: &str, job_name: &str, result: Option<DerivativeReference>) {
        let work = {
            let mut state = self.state.lock().await;
            if state
                .by_key
                .get(key)
                .is_some_and(|work| work.job_name == job_name)
            {
                state.by_key.remove(key)
            } else {
                None
            }
        };
        if let Some(work) = work {
            for waiter in work.waiters {
                let _ = waiter.send(result.clone());
            }
        }
    }

    pub(crate) async fn invalidate_except(&self, selection: SelectionToken) {
        let mut state = self.state.lock().await;
        let stale = state
            .by_key
            .iter()
            .filter(|(_, work)| work.pending.selection != selection)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for key in stale {
            if let Some(work) = state.by_key.remove(&key) {
                state.job_to_key.remove(&work.job_name);
            }
        }
    }

    async fn is_empty(&self) -> bool {
        self.state.lock().await.by_key.is_empty()
    }
}

struct WallRequestGuard {
    service: AppService,
    selection: SelectionToken,
    generation: u64,
    request_id: u64,
}

impl Drop for WallRequestGuard {
    fn drop(&mut self) {
        self.service
            .finish_wall_request(self.selection, self.generation, self.request_id);
    }
}

enum PreviewPrefetchOutcome {
    Complete,
    Blocked,
    Stale,
    Terminate,
}

impl AppService {
    pub async fn set_interaction(&self, state: crate::InteractionState) {
        self.scheduler
            .set_interaction_mode(match state {
                crate::InteractionState::Idle => InteractionMode::Idle,
                crate::InteractionState::Active => InteractionMode::Active,
            })
            .await;
        if state == crate::InteractionState::Idle && !self.derivative_queue.is_empty().await {
            let service = self.clone();
            tokio::spawn(async move {
                service.drive_derivative_queue(true).await;
            });
        }
        let recent = self
            .state()
            .map(|state| state.recent_derivative_ids.clone())
            .unwrap_or_default();
        self.schedule_screen_preview_prefetch(recent);
    }

    pub async fn request_derivatives(
        &self,
        request: DerivativeRequest,
    ) -> Result<(), AppServiceError> {
        if request.priority == DerivativePriority::Visible
            && request.kind == DerivativeClass::WallThumbnail
        {
            self.fence_visible_wall_request();
            #[cfg(debug_assertions)]
            if let Some(hook) = self.derivative_request_test_hook.lock().await.clone() {
                hook.started.notify_one();
                if let Some(release) = hook.release {
                    release.notified().await;
                }
            }
        }
        if request.asset_ids.len() > 250 {
            return Err(AppServiceError::InvalidLimit);
        }
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
        let _wall_request = (request.kind == DerivativeClass::WallThumbnail)
            .then(|| self.begin_preview_wall_request(request_selection));
        let priority = match request.priority {
            DerivativePriority::Visible => JobPriority::Visible,
            DerivativePriority::NearViewport => JobPriority::NearViewport,
        };
        let derivative_ids = if request.kind == DerivativeClass::ScreenPreview {
            let (_, wall_ready) = self.ensure_wall_thumbnails(&ids, priority).await?;
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
        if request.kind == DerivativeClass::WallThumbnail
            && let Ok(mut state) = self.state.lock()
            && state.selection_epoch == selection.epoch
            && state.protected_group == Some(selection.group_id)
        {
            append_unique(&mut state.recent_derivative_ids, ids.iter().copied());
        }
        let pending_ids = pending
            .iter()
            .map(|work| work.id.as_uuid().hyphenated().to_string())
            .collect::<HashSet<_>>();
        let generated = self.run_derivative_jobs_publishing(pending, priority).await;
        successful_ids.extend(generated.iter().map(|reference| reference.asset_id.clone()));
        let has_unresolved = pending_ids.difference(&successful_ids).next().is_some();
        let successful_in_request = derivative_ids
            .iter()
            .copied()
            .filter(|id| successful_ids.contains(&id.as_uuid().hyphenated().to_string()))
            .collect::<Vec<_>>();
        if request.kind == DerivativeClass::WallThumbnail {
            self.schedule_screen_preview_prefetch(successful_in_request);
        }
        if has_unresolved {
            return Err(AppServiceError::DerivativeUnavailable);
        }
        Ok(())
    }

    async fn ensure_wall_thumbnails(
        &self,
        ids: &[AssetId],
        priority: JobPriority,
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
        let generated = self.run_derivative_jobs_publishing(pending, priority).await;
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
        self.resolve_derivatives_with_generation(ids, class, None)
    }

    fn resolve_screen_previews(
        &self,
        ids: &[AssetId],
        generation: u64,
    ) -> Result<
        (
            SelectionToken,
            Vec<DerivativeReference>,
            Vec<PendingDerivative>,
        ),
        AppServiceError,
    > {
        self.resolve_derivatives_with_generation(
            ids,
            DerivativeClass::ScreenPreview,
            Some(generation),
        )
    }

    fn resolve_derivatives_with_generation(
        &self,
        ids: &[AssetId],
        class: DerivativeClass,
        preview_generation: Option<u64>,
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
        let kind = derivative_kind_name(class);
        let records = state
            .libraries
            .catalog()
            .derivatives_for_assets(ids, kind)?;
        let prerequisite_records = match class {
            DerivativeClass::WallThumbnail => state.libraries.catalog().derivatives_for_assets(
                ids,
                derivative_kind_name(DerivativeClass::ScreenPreview),
            )?,
            DerivativeClass::ScreenPreview => state.libraries.catalog().derivatives_for_assets(
                ids,
                derivative_kind_name(DerivativeClass::WallThumbnail),
            )?,
        };
        let mut ready = Vec::new();
        let mut pending = Vec::new();
        for id in ids {
            let asset = state
                .libraries
                .catalog()
                .find_asset(*id)?
                .ok_or(AppServiceError::UnknownAsset)?;
            if asset.folder_group_id != Some(group) {
                return Err(AppServiceError::ForeignAsset);
            }
            let spec = derivative_spec(&asset, class);
            let key = DerivativeKey::compute(&spec);
            if class == DerivativeClass::ScreenPreview {
                let wall_spec = derivative_spec(&asset, DerivativeClass::WallThumbnail);
                let wall_key = DerivativeKey::compute(&wall_spec);
                let wall_ready = prerequisite_records.iter().any(|record| {
                    record.asset_id == *id
                        && record.folder_group_id == group
                        && record.cache_key == wall_key.as_str()
                });
                if !wall_ready {
                    continue;
                }
            }
            if let Some(existing) = records.iter().find(|record| {
                record.asset_id == *id
                    && record.folder_group_id == group
                    && record.cache_key == key.as_str()
            }) {
                ready.push(reference(*id, class, existing.cache_key.clone()));
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
                            record.asset_id == *id
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
                        id: *id,
                        source: root.join(relative),
                        cached_source,
                        spec,
                        group,
                        class,
                        selection: selection_token,
                        preview_generation,
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
        self.run_derivative_jobs_with_publication(pending, priority, false)
            .await
    }

    async fn run_derivative_jobs_publishing(
        &self,
        pending: Vec<PendingDerivative>,
        priority: JobPriority,
    ) -> Vec<DerivativeReference> {
        self.run_derivative_jobs_with_publication(pending, priority, true)
            .await
    }

    async fn run_derivative_jobs_with_publication(
        &self,
        pending: Vec<PendingDerivative>,
        priority: JobPriority,
        publish: bool,
    ) -> Vec<DerivativeReference> {
        let mut receivers = Vec::with_capacity(pending.len());
        for pending in pending {
            receivers.push(
                self.derivative_queue
                    .enqueue(&self.scheduler, pending, priority)
                    .await,
            );
        }
        if !receivers.is_empty() {
            self.wake_preview_gate();
            #[cfg(debug_assertions)]
            if priority == JobPriority::Visible
                && let Some(hook) = self.derivative_visible_queue_test_hook.lock().await.clone()
            {
                hook.notify_one();
            }
            let service = self.clone();
            tokio::spawn(async move {
                service.drive_derivative_queue(publish).await;
            });
        }
        let mut ready = Vec::new();
        for receiver in receivers {
            if let Ok(Some(reference)) = receiver.await {
                ready.push(reference);
            }
        }
        ready
    }

    async fn drive_derivative_queue(&self, publish: bool) {
        let _driver = self.derivative_queue.driver.lock().await;
        let worker_count = self
            .scheduler
            .available_background_permits()
            .clamp(1, MAX_DERIVATIVE_WORKERS);
        let mut workers = Vec::with_capacity(worker_count);
        for _ in 0..worker_count {
            let service = self.clone();
            workers.push(tokio::spawn(async move {
                service.drive_derivative_worker(publish).await;
            }));
        }
        for worker in workers {
            let _ = worker.await;
        }
    }

    async fn drive_derivative_worker(&self, publish: bool) {
        loop {
            let Some((job, key, pending)) = self.derivative_queue.next_owned(&self.scheduler).await
            else {
                break;
            };
            if should_pause(
                job.priority(),
                self.scheduler.available_background_permits(),
            ) {
                self.derivative_queue.requeue(&job, &key).await;
                self.scheduler.enqueue(job).await;
                break;
            }
            let selection = pending.selection;
            let asset_id = pending.id;
            let class = pending.class;
            let preview_generation = pending.preview_generation;
            if class == DerivativeClass::ScreenPreview
                && preview_generation
                    .is_some_and(|generation| !self.preview_generation_is_current(generation))
            {
                self.derivative_queue.finish(&key, job.name(), None).await;
                self.wake_preview_gate();
                continue;
            }
            let result = if self.selection_is_active(selection) {
                match self.generate_derivative(pending).await {
                    Ok(reference) => {
                        self.clear_derivative_warnings(selection, asset_id, class);
                        if publish {
                            self.publish_derivatives_if_active(selection, vec![reference.clone()]);
                        }
                        Some(reference)
                    }
                    Err(DerivativeWorkError::PreviewSuperseded) => None,
                    Err(DerivativeWorkError::WallThumbnailRequired) => None,
                    Err(error) => {
                        match error.scope() {
                            DerivativeFailureScope::Asset => {
                                self.record_asset_derivative_warning(selection, asset_id);
                            }
                            DerivativeFailureScope::CacheWide => {
                                self.record_cache_derivative_warning(selection, class);
                            }
                        }
                        None
                    }
                }
            } else {
                None
            };
            self.derivative_queue.finish(&key, job.name(), result).await;
            self.wake_preview_gate();
        }
    }

    async fn generate_derivative(
        &self,
        pending: PendingDerivative,
    ) -> Result<DerivativeReference, DerivativeWorkError> {
        let id = pending.id;
        let group = pending.group;
        let class = pending.class;
        let selection = pending.selection;
        #[cfg(any(test, debug_assertions))]
        self.wait_for_test_derivative_gate(id, class).await;
        if class == DerivativeClass::ScreenPreview
            && pending
                .preview_generation
                .is_some_and(|generation| !self.preview_generation_is_current(generation))
        {
            return Err(DerivativeWorkError::PreviewSuperseded);
        }
        if !self.selection_is_active(selection) {
            return Err(DerivativeWorkError::WorkerUnavailable);
        }
        if class == DerivativeClass::ScreenPreview {
            let wall_spec = wall_spec_from(&pending.spec);
            let wall_key = DerivativeKey::compute(&wall_spec);
            let wall_ready = self
                .state()
                .map_err(|_| DerivativeWorkError::WorkerUnavailable)?
                .libraries
                .catalog()
                .find_derivative(
                    id,
                    derivative_kind_name(DerivativeClass::WallThumbnail),
                    wall_key.as_str(),
                )
                .map_err(|_| DerivativeWorkError::WorkerUnavailable)?
                .is_some_and(|record| record.folder_group_id == group);
            if !wall_ready {
                return Err(DerivativeWorkError::WallThumbnailRequired);
            }
        }
        let cached_screen_preview = if class == DerivativeClass::WallThumbnail {
            pending
                .cached_source
                .clone()
                .or(self.cached_screen_preview_for_wall(id, group, &pending.spec)?)
        } else {
            None
        };
        let generator = ImageDerivativeGenerator::new(&self.cache_root)?;
        let generated = match class {
            DerivativeClass::WallThumbnail => {
                let source = pending.source;
                let cached_source = cached_screen_preview;
                let spec = pending.spec;
                tokio::task::spawn_blocking(move || {
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
                .map_err(|_| DerivativeWorkError::WorkerUnavailable)??
            }
            DerivativeClass::ScreenPreview => {
                let source = pending.source;
                let spec = pending.spec;
                let catalog_path = self.catalog_path.clone();
                let cache_budget = self.cache_budget;
                let protected = self.protected_groups.clone();
                let encoded = tokio::task::spawn_blocking({
                    let generator = generator.clone();
                    let spec = spec.clone();
                    move || generator.encode_screen_preview(&source, &spec)
                })
                .await
                .map_err(|_| DerivativeWorkError::WorkerUnavailable)??;
                if !self.selection_is_active(selection) {
                    return Err(DerivativeWorkError::WorkerUnavailable);
                }
                let service = self.clone();
                tokio::task::spawn_blocking(move || {
                    service.commit_screen_preview_if_active(
                        generator,
                        encoded,
                        &spec,
                        group,
                        cache_budget,
                        &protected,
                        selection,
                        &catalog_path,
                    )
                })
                .await
                .map_err(|_| DerivativeWorkError::WorkerUnavailable)??
            }
        };
        if !self.selection_is_active(selection) {
            return Err(DerivativeWorkError::WorkerUnavailable);
        }
        if class == DerivativeClass::WallThumbnail {
            let mut state = self
                .state()
                .map_err(|_| DerivativeWorkError::WorkerUnavailable)?;
            let catalog_started = std::time::Instant::now();
            let catalog_result = state
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
            catalog_result.map_err(ImageDerivativeError::from)?;
        }
        Ok(reference(id, class, generated.key.as_str().into()))
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

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn reset_preview_gate_test_checks(&self) {
        self.preview_gate_checks.store(0, Ordering::SeqCst);
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn preview_gate_test_checks(&self) -> usize {
        self.preview_gate_checks.load(Ordering::SeqCst)
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn preview_gate_generation_test(&self) -> u64 {
        self.preview_gate
            .lock()
            .map(|gate| gate.generation)
            .unwrap_or_default()
    }

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
        let _commit_guard = self
            .screen_preview_commit_lock
            .lock()
            .map_err(|_| DerivativeWorkError::WorkerUnavailable)?;
        if !self.selection_is_active(selection) {
            return Err(DerivativeWorkError::WorkerUnavailable);
        }
        let mut catalog = Catalog::open(catalog_path).map_err(ImageDerivativeError::from)?;
        generator
            .commit_screen_preview(encoded, spec, group, &mut catalog, budget, protected)
            .map_err(Into::into)
    }

    pub(crate) async fn prefetch_screen_previews(&self, recent: Vec<AssetId>) {
        self.schedule_screen_preview_prefetch_for_group(recent, true);
    }

    pub(crate) fn wake_preview_gate(&self) {
        self.preview_gate_wake.notify_one();
    }

    fn schedule_screen_preview_prefetch(&self, recent: Vec<AssetId>) {
        self.schedule_screen_preview_prefetch_for_group(recent, false);
    }

    fn schedule_screen_preview_prefetch_for_group(&self, recent: Vec<AssetId>, full_group: bool) {
        let stable_recent = self
            .state()
            .map(|state| state.recent_derivative_ids.clone())
            .unwrap_or_default();
        let should_spawn = {
            let Ok(mut gate) = self.preview_gate.lock() else {
                return;
            };
            append_unique(&mut gate.recent_ids, stable_recent);
            append_unique(&mut gate.recent_ids, recent);
            gate.full_group_prefetch |= full_group;
            gate.generation = gate.generation.wrapping_add(1).max(1);
            if gate.task_running || (gate.recent_ids.is_empty() && !gate.full_group_prefetch) {
                false
            } else {
                gate.task_running = true;
                true
            }
        };
        self.preview_gate_wake.notify_one();
        if !should_spawn {
            return;
        }
        if tokio::runtime::Handle::try_current().is_err() {
            if let Ok(mut gate) = self.preview_gate.lock() {
                gate.task_running = false;
            }
            return;
        }
        let service = self.clone();
        tokio::spawn(async move {
            service.run_preview_gate().await;
        });
    }

    pub(crate) fn reset_preview_gate(&self) {
        if let Ok(mut gate) = self.preview_gate.lock() {
            gate.recent_ids.clear();
            gate.full_group_prefetch = false;
            gate.generation = gate.generation.wrapping_add(1).max(1);
            gate.wall_requests_in_flight = 0;
            gate.wall_request_selection = None;
            gate.wall_request_generation = gate.wall_request_generation.wrapping_add(1).max(1);
            gate.active_wall_request_ids.clear();
        }
        self.wake_preview_gate();
    }

    fn fence_visible_wall_request(&self) {
        if let Ok(mut gate) = self.preview_gate.lock() {
            gate.generation = gate.generation.wrapping_add(1).max(1);
        }
        self.wake_preview_gate();
    }

    fn begin_preview_wall_request(&self, selection: SelectionToken) -> WallRequestGuard {
        let (generation, request_id) = {
            let mut gate = self
                .preview_gate
                .lock()
                .expect("preview gate lock must not be poisoned");
            if gate.wall_request_selection != Some(selection) {
                gate.active_wall_request_ids.clear();
                gate.wall_requests_in_flight = 0;
                gate.wall_request_selection = Some(selection);
                gate.wall_request_generation = gate.wall_request_generation.wrapping_add(1).max(1);
            }
            gate.next_wall_request_id = gate.next_wall_request_id.wrapping_add(1).max(1);
            let request_id = gate.next_wall_request_id;
            gate.active_wall_request_ids.insert(request_id);
            gate.wall_requests_in_flight = gate.active_wall_request_ids.len();
            gate.generation = gate.generation.wrapping_add(1).max(1);
            (gate.wall_request_generation, request_id)
        };
        self.wake_preview_gate();
        WallRequestGuard {
            service: self.clone(),
            selection,
            generation,
            request_id,
        }
    }

    fn finish_wall_request(&self, selection: SelectionToken, generation: u64, request_id: u64) {
        if let Ok(mut gate) = self.preview_gate.lock()
            && gate.wall_request_selection == Some(selection)
            && gate.wall_request_generation == generation
            && gate.active_wall_request_ids.remove(&request_id)
        {
            gate.wall_requests_in_flight = gate.active_wall_request_ids.len();
            gate.generation = gate.generation.wrapping_add(1).max(1);
        }
        self.wake_preview_gate();
    }

    async fn run_preview_gate(&self) {
        loop {
            let (generation, recent) = {
                let Ok(mut gate) = self.preview_gate.lock() else {
                    return;
                };
                if gate.recent_ids.is_empty() && !gate.full_group_prefetch {
                    gate.task_running = false;
                    return;
                }
                (gate.generation, gate.recent_ids.clone())
            };

            tokio::select! {
                _ = tokio::time::sleep(PREVIEW_DEBOUNCE) => {}
                _ = self.preview_gate_wake.notified() => continue,
            }

            let wake = self.preview_gate_wake.notified();
            if self.preview_selection_if_ready(generation).await.is_none() {
                wake.await;
                continue;
            }
            match self
                .run_screen_preview_prefetch_for_generation(recent, generation)
                .await
            {
                PreviewPrefetchOutcome::Complete => {
                    let should_continue = {
                        let Ok(mut gate) = self.preview_gate.lock() else {
                            return;
                        };
                        if gate.generation == generation {
                            gate.recent_ids.clear();
                            gate.full_group_prefetch = false;
                            gate.task_running = false;
                            false
                        } else {
                            true
                        }
                    };
                    if !should_continue {
                        return;
                    }
                }
                PreviewPrefetchOutcome::Blocked => {
                    self.preview_gate_wake.notified().await;
                }
                PreviewPrefetchOutcome::Stale => {}
                PreviewPrefetchOutcome::Terminate => {
                    let Ok(mut gate) = self.preview_gate.lock() else {
                        return;
                    };
                    if gate.generation == generation {
                        gate.recent_ids.clear();
                        gate.full_group_prefetch = false;
                        gate.task_running = false;
                        return;
                    }
                }
            }
        }
    }

    async fn preview_selection_if_ready(&self, generation: u64) -> Option<SelectionToken> {
        if self
            .preview_gate
            .lock()
            .ok()
            .is_none_or(|gate| gate.generation != generation)
        {
            return None;
        }
        let selection = self.active_selection_token().ok()?;
        if !self.preview_work_can_continue(selection).await {
            return None;
        }
        Some(selection)
    }

    async fn preview_work_can_continue(&self, selection: SelectionToken) -> bool {
        #[cfg(debug_assertions)]
        self.preview_gate_checks.fetch_add(1, Ordering::SeqCst);
        if self
            .preview_gate
            .lock()
            .map(|gate| gate.wall_requests_in_flight > 0)
            .unwrap_or(true)
        {
            return false;
        }
        if !self.selection_is_active(selection)
            || self
                .state()
                .map(|state| state.active_scan.is_some())
                .unwrap_or(true)
            || self.scheduler.available_background_permits() <= 1
        {
            return false;
        }
        self.derivative_queue.is_empty().await
    }

    #[cfg(test)]
    async fn run_screen_preview_prefetch(&self, recent: Vec<AssetId>) {
        let generation = self
            .preview_gate
            .lock()
            .map(|gate| gate.generation)
            .unwrap_or_default();
        let _ = self
            .run_screen_preview_prefetch_for_generation(recent, generation)
            .await;
    }

    async fn run_screen_preview_prefetch_for_generation(
        &self,
        recent: Vec<AssetId>,
        generation: u64,
    ) -> PreviewPrefetchOutcome {
        if !self.preview_generation_is_current(generation) {
            return PreviewPrefetchOutcome::Stale;
        }
        let Ok(selection) = self.active_selection_token() else {
            return PreviewPrefetchOutcome::Terminate;
        };
        if !self.preview_work_can_continue(selection).await {
            return PreviewPrefetchOutcome::Blocked;
        }
        let Ok((selection, wall_ready)) = self
            .ensure_wall_thumbnails(&recent, JobPriority::NearViewport)
            .await
        else {
            return PreviewPrefetchOutcome::Terminate;
        };
        if !self.preview_generation_is_current(generation) {
            return PreviewPrefetchOutcome::Stale;
        }
        let recent_ready = recent
            .iter()
            .copied()
            .filter(|id| wall_ready.contains(id))
            .collect::<Vec<_>>();
        let Ok((screen_selection, cached, pending)) =
            self.resolve_screen_previews(&recent_ready, generation)
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
            .run_derivative_jobs_publishing(pending, JobPriority::NearViewport)
            .await;
        if !self.preview_generation_is_current(generation) {
            return PreviewPrefetchOutcome::Stale;
        }

        let recent = recent.into_iter().collect::<HashSet<_>>();
        let mut cursor = None;
        loop {
            if !self.preview_generation_is_current(generation) {
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
                    .ensure_wall_thumbnails(&remaining, JobPriority::IdleLibrary)
                    .await
                else {
                    return PreviewPrefetchOutcome::Terminate;
                };
                if wall_selection != selection {
                    return PreviewPrefetchOutcome::Stale;
                }
                if !self.preview_generation_is_current(generation) {
                    return PreviewPrefetchOutcome::Stale;
                }
                let screen_ids = remaining
                    .iter()
                    .copied()
                    .filter(|id| wall_ready.contains(id))
                    .collect::<Vec<_>>();
                let Ok((resolved_selection, cached, pending)) =
                    self.resolve_screen_previews(&screen_ids, generation)
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
                    .run_derivative_jobs_publishing(pending, JobPriority::IdleLibrary)
                    .await;
                if !self.preview_generation_is_current(generation) {
                    return PreviewPrefetchOutcome::Stale;
                }
            }
            let Some(next) = next else {
                break;
            };
            cursor = Some(next);
        }
        PreviewPrefetchOutcome::Complete
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

    fn preview_generation_is_current(&self, generation: u64) -> bool {
        self.preview_gate
            .lock()
            .map(|gate| gate.generation == generation)
            .unwrap_or(false)
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

fn append_unique<I>(target: &mut Vec<AssetId>, ids: I)
where
    I: IntoIterator<Item = AssetId>,
{
    for id in ids {
        if !target.contains(&id) {
            target.push(id);
        }
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
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Arc;

    use image::{ImageBuffer, Rgb};
    use photo_cache::{
        CacheBudget, DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget,
        EncodedScreenPreview, ImageDerivativeGenerator,
    };
    use photo_catalog::{AssetShapeUpdate, Catalog, CatalogIndexRecord, NewAsset, ShapeStatus};
    use photo_domain::{
        AssetId, FileSignature, FolderGroupId, LibraryId, MediaKind, RelativePathKey,
    };
    use photo_indexer::{IndexJob, IndexScheduler, JobPriority, SchedulerConfig};

    use crate::service::{DerivativeTestGate, SelectionToken};
    use crate::{AppConfig, AppService, DerivativeClass, DerivativeReference, WallUpdate};

    use super::{DerivativeQueue, DerivativeWorkError, PendingDerivative};

    #[tokio::test]
    async fn derivative_driver_leaves_foreign_scheduler_jobs_available() {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig::default()));
        scheduler
            .enqueue(IndexJob::new("foreign", JobPriority::Visible))
            .await;
        let queue = DerivativeQueue::default();

        assert!(queue.next_owned(&scheduler).await.is_none());
        assert_eq!(scheduler.next().await.unwrap().name(), "foreign");
    }

    #[tokio::test]
    async fn concurrent_producers_share_one_immutable_derivative_job() {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig::default()));
        let queue = DerivativeQueue::default();
        let asset_id = AssetId::from_uuid(uuid::Uuid::new_v4());
        let group_id = FolderGroupId::new();
        let selection = SelectionToken {
            library_id: LibraryId::new(),
            group_id,
            epoch: 4,
        };
        let spec = DerivativeSpec {
            asset_id,
            signature: FileSignature {
                size_bytes: 10,
                modified_unix_ns: 20,
                sidecar_modified_unix_ns: None,
            },
            orientation: 1,
            kind: DerivativeKind::ScreenPreview,
            decoder_version: "decoder".to_owned(),
            colour_space: "srgb".to_owned(),
            target: DerivativeTarget::LongEdge(4096),
        };
        let pending = PendingDerivative {
            id: asset_id,
            source: PathBuf::from("source.jpg"),
            cached_source: None,
            spec: spec.clone(),
            group: group_id,
            class: DerivativeClass::ScreenPreview,
            selection,
            preview_generation: None,
        };

        let first = queue
            .enqueue(&scheduler, pending.clone(), JobPriority::NearViewport)
            .await;
        let second = queue
            .enqueue(&scheduler, pending, JobPriority::IdleLibrary)
            .await;
        let (_job, key, queued) = queue.next_owned(&scheduler).await.unwrap();

        assert_eq!(queued.id, asset_id);
        assert!(scheduler.next().await.is_none());
        assert_eq!(key, DerivativeKey::compute(&spec).as_str());
        let reference = DerivativeReference {
            asset_id: asset_id.as_uuid().hyphenated().to_string(),
            kind: DerivativeClass::ScreenPreview,
            key: key.clone(),
        };
        queue
            .finish(&key, _job.name(), Some(reference.clone()))
            .await;
        assert_eq!(first.await.unwrap(), Some(reference.clone()));
        assert_eq!(second.await.unwrap(), Some(reference));
    }

    #[tokio::test]
    async fn concurrent_workers_take_distinct_jobs_in_priority_order() {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig::default()));
        let queue = Arc::new(DerivativeQueue::default());
        let selection = SelectionToken {
            library_id: LibraryId::new(),
            group_id: FolderGroupId::new(),
            epoch: 1,
        };
        let pending = |asset_id| PendingDerivative {
            id: asset_id,
            source: PathBuf::from("source.jpg"),
            cached_source: None,
            spec: DerivativeSpec {
                asset_id,
                signature: photo_domain::FileSignature {
                    size_bytes: 1,
                    modified_unix_ns: 1,
                    sidecar_modified_unix_ns: None,
                },
                orientation: 1,
                kind: DerivativeKind::WallThumbnail,
                decoder_version: "decoder".to_owned(),
                colour_space: "srgb".to_owned(),
                target: DerivativeTarget::LongEdge(1024),
            },
            group: selection.group_id,
            class: DerivativeClass::WallThumbnail,
            selection,
            preview_generation: None,
        };
        let first_id = AssetId::from_uuid(uuid::Uuid::new_v4());
        let second_id = AssetId::from_uuid(uuid::Uuid::new_v4());
        let first = queue
            .enqueue(&scheduler, pending(first_id), JobPriority::NearViewport)
            .await;
        let second = queue
            .enqueue(&scheduler, pending(second_id), JobPriority::Visible)
            .await;

        let left = {
            let queue = queue.clone();
            let scheduler = scheduler.clone();
            tokio::spawn(async move { queue.next_owned(&scheduler).await.unwrap() })
        };
        let right = {
            let queue = queue.clone();
            let scheduler = scheduler.clone();
            tokio::spawn(async move { queue.next_owned(&scheduler).await.unwrap() })
        };
        let left = left.await.unwrap();
        let right = right.await.unwrap();
        assert!(
            (left.0.priority() == JobPriority::Visible
                && right.0.priority() == JobPriority::NearViewport)
                || (right.0.priority() == JobPriority::Visible
                    && left.0.priority() == JobPriority::NearViewport)
        );
        assert_ne!(left.2.id, right.2.id);
        assert_eq!(
            [left.2.id, right.2.id].into_iter().collect::<HashSet<_>>(),
            [first_id, second_id].into_iter().collect()
        );
        drop(first);
        drop(second);
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

    #[tokio::test]
    async fn stale_in_flight_completion_cannot_consume_replacement_selection_work() {
        let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig::default()));
        let queue = DerivativeQueue::default();
        let asset_id = AssetId::from_uuid(uuid::Uuid::new_v4());
        let group_id = FolderGroupId::new();
        let library_id = LibraryId::new();
        let spec = DerivativeSpec {
            asset_id,
            signature: FileSignature {
                size_bytes: 10,
                modified_unix_ns: 20,
                sidecar_modified_unix_ns: None,
            },
            orientation: 1,
            kind: DerivativeKind::ScreenPreview,
            decoder_version: "decoder".to_owned(),
            colour_space: "srgb".to_owned(),
            target: DerivativeTarget::LongEdge(4096),
        };
        let pending = |epoch| PendingDerivative {
            id: asset_id,
            source: PathBuf::from("source.jpg"),
            cached_source: None,
            spec: spec.clone(),
            group: group_id,
            class: DerivativeClass::ScreenPreview,
            selection: SelectionToken {
                library_id,
                group_id,
                epoch,
            },
            preview_generation: None,
        };
        let stale_receiver = queue
            .enqueue(&scheduler, pending(1), JobPriority::IdleLibrary)
            .await;
        let (stale_job, key, _) = queue.next_owned(&scheduler).await.unwrap();
        let current_receiver = queue
            .enqueue(&scheduler, pending(2), JobPriority::Visible)
            .await;

        queue.finish(&key, stale_job.name(), None).await;
        assert!(stale_receiver.await.is_err());
        let (current_job, current_key, _) = queue.next_owned(&scheduler).await.unwrap();
        let reference = DerivativeReference {
            asset_id: asset_id.as_uuid().hyphenated().to_string(),
            kind: DerivativeClass::ScreenPreview,
            key: current_key.clone(),
        };
        queue
            .finish(&current_key, current_job.name(), Some(reference.clone()))
            .await;
        assert_eq!(current_receiver.await.unwrap(), Some(reference));
    }

    #[test]
    fn stale_wall_request_guard_cannot_consume_replacement_selection_work() {
        let temp = tempfile::tempdir().unwrap();
        let first_source = temp.path().join("first");
        let second_source = temp.path().join("second");
        std::fs::create_dir_all(&first_source).unwrap();
        std::fs::create_dir_all(&second_source).unwrap();
        let service = AppService::open(AppConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
        ))
        .unwrap();
        let (_, first_selection) = service.select_recent(&first_source).unwrap();
        let stale = service.begin_preview_wall_request(first_selection);
        let (_, replacement_selection) = service.select_recent(&second_source).unwrap();
        let current = service.begin_preview_wall_request(replacement_selection);

        drop(stale);
        let gate = service.preview_gate.lock().unwrap();
        assert_eq!(gate.wall_requests_in_flight, 1);
        assert_eq!(gate.wall_request_selection, Some(replacement_selection));
        drop(gate);
        drop(current);
        assert_eq!(
            service.preview_gate.lock().unwrap().wall_requests_in_flight,
            0
        );
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
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            while !service.derivative_queue.is_empty().await {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        service.reset_preview_gate();
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                let task_running = service
                    .preview_gate
                    .lock()
                    .map(|gate| gate.task_running)
                    .unwrap_or(true);
                if !task_running && service.derivative_queue.is_empty().await {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
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
