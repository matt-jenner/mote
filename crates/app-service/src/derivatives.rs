//! Derivative request orchestration lives here so source paths never cross the DTO boundary.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

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

#[derive(Debug)]
enum DerivativeWorkError {
    Image(ImageDerivativeError),
    WorkerUnavailable,
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
            | Self::WorkerUnavailable => DerivativeFailureScope::CacheWide,
        }
    }
}

#[derive(Clone)]
pub(crate) struct PendingDerivative {
    id: AssetId,
    source: PathBuf,
    spec: DerivativeSpec,
    group: FolderGroupId,
    class: DerivativeClass,
    selection: SelectionToken,
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

impl AppService {
    pub async fn set_interaction(&self, state: crate::InteractionState) {
        self.scheduler
            .set_interaction_mode(match state {
                crate::InteractionState::Idle => InteractionMode::Idle,
                crate::InteractionState::Active => InteractionMode::Active,
            })
            .await;
        if state == crate::InteractionState::Idle {
            let service = self.clone();
            tokio::spawn(async move {
                let has_paused_jobs = !service.derivative_queue.is_empty().await;
                if has_paused_jobs {
                    service.drive_derivative_queue().await;
                } else {
                    service.prefetch_screen_previews(Vec::new()).await;
                }
            });
        }
    }

    pub async fn request_derivatives(
        &self,
        request: DerivativeRequest,
    ) -> Result<(), AppServiceError> {
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

        let (selection, ready, pending) =
            self.resolve_derivatives(&ids, DerivativeClass::WallThumbnail)?;
        let priority = match request.priority {
            DerivativePriority::Visible => JobPriority::Visible,
            DerivativePriority::NearViewport => JobPriority::NearViewport,
        };
        let generated = self.run_derivative_jobs(pending, priority).await;
        let derivatives = merge_in_request_order(&ids, ready, generated);
        self.publish_derivatives_if_active(selection, derivatives);

        if let Ok(mut state) = self.state.lock()
            && state.selection_epoch == selection.epoch
            && state.protected_group == Some(selection.group_id)
        {
            state.recent_derivative_ids = ids.clone();
        }

        let service = self.clone();
        tokio::spawn(async move {
            service.prefetch_screen_previews(ids).await;
        });
        Ok(())
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
            if let Some(existing) = records
                .iter()
                .find(|record| record.asset_id == *id && record.cache_key == key.as_str())
            {
                ready.push(reference(*id, class, existing.cache_key.clone()));
            } else if matches!(asset.availability, Availability::Available) {
                let Some(relative) = asset.relative_path.to_path_buf().ok() else {
                    continue;
                };
                pending.push(PendingDerivative {
                    id: *id,
                    source: root.join(relative),
                    spec,
                    group,
                    class,
                    selection: selection_token,
                });
            }
        }
        Ok((selection_token, ready, pending))
    }

    async fn run_derivative_jobs(
        &self,
        pending: Vec<PendingDerivative>,
        priority: JobPriority,
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
            let service = self.clone();
            tokio::spawn(async move {
                service.drive_derivative_queue().await;
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

    async fn drive_derivative_queue(&self) {
        let _driver = self.derivative_queue.driver.lock().await;
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
            let result = if self.selection_is_active(selection) {
                match self.generate_derivative(pending).await {
                    Ok(reference) => {
                        self.clear_derivative_warnings(selection, asset_id, class);
                        Some(reference)
                    }
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
        }
    }

    async fn generate_derivative(
        &self,
        pending: PendingDerivative,
    ) -> Result<DerivativeReference, DerivativeWorkError> {
        let generator = ImageDerivativeGenerator::new(&self.cache_root)?;
        let id = pending.id;
        let group = pending.group;
        let class = pending.class;
        let selection = pending.selection;
        let generated = match class {
            DerivativeClass::WallThumbnail => {
                let source = pending.source;
                let spec = pending.spec;
                tokio::task::spawn_blocking(move || generator.generate(&source, &spec))
                    .await
                    .map_err(|_| DerivativeWorkError::WorkerUnavailable)??
            }
            DerivativeClass::ScreenPreview => {
                let source = pending.source;
                let spec = pending.spec;
                let catalog_path = self.catalog_path.clone();
                let cache_budget = self.cache_budget;
                let protected = self.protected_groups.clone();
                tokio::task::spawn_blocking(move || {
                    let mut catalog = Catalog::open(&catalog_path)?;
                    generator.generate_screen_preview(
                        &source,
                        spec.asset_id,
                        spec.signature,
                        spec.orientation,
                        group,
                        &mut catalog,
                        cache_budget,
                        &protected,
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
                })
                .map_err(ImageDerivativeError::from)?;
        }
        Ok(reference(id, class, generated.key.as_str().into()))
    }

    pub(crate) async fn prefetch_screen_previews(&self, recent: Vec<AssetId>) {
        if self
            .state()
            .map(|state| state.active_scan.is_some())
            .unwrap_or(true)
        {
            return;
        }
        let Ok((selection, cached, pending)) =
            self.resolve_derivatives(&recent, DerivativeClass::ScreenPreview)
        else {
            return;
        };
        let generated = self
            .run_derivative_jobs(pending, JobPriority::NearViewport)
            .await;
        let derivatives = merge_in_request_order(&recent, cached, generated);
        if !derivatives.is_empty() {
            self.publish_derivatives_if_active(selection, derivatives);
        }

        let recent = recent.into_iter().collect::<HashSet<_>>();
        let mut cursor = None;
        loop {
            if self.scheduler.available_background_permits() <= 1
                || !self.selection_is_active(selection)
            {
                return;
            }
            let Ok((page_selection, remaining, next)) =
                self.remaining_group_ids_page(&recent, cursor)
            else {
                return;
            };
            if page_selection != selection {
                return;
            }
            if !remaining.is_empty() {
                let Ok((resolved_selection, _cached, pending)) =
                    self.resolve_derivatives(&remaining, DerivativeClass::ScreenPreview)
                else {
                    return;
                };
                if resolved_selection != selection {
                    return;
                }
                let derivatives = self
                    .run_derivative_jobs(pending, JobPriority::IdleLibrary)
                    .await;
                if !derivatives.is_empty() {
                    self.publish_derivatives_if_active(selection, derivatives);
                }
            }
            let Some(next) = next else {
                break;
            };
            cursor = Some(next);
        }
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
        let _ = self.updates.send(WallUpdate::DerivativesReady {
            selection_id: selection.selection_id(),
            derivatives,
        });
        true
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

fn merge_in_request_order(
    ids: &[AssetId],
    ready: Vec<DerivativeReference>,
    generated: Vec<DerivativeReference>,
) -> Vec<DerivativeReference> {
    let references = ready
        .into_iter()
        .chain(generated)
        .map(|item| (item.asset_id.clone(), item))
        .collect::<HashMap<_, _>>();
    ids.iter()
        .filter_map(|id| {
            references
                .get(&id.as_uuid().hyphenated().to_string())
                .cloned()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::Arc;

    use image::{ImageBuffer, Rgb};
    use photo_cache::{
        CacheBudget, DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget,
    };
    use photo_catalog::{AssetShapeUpdate, CatalogIndexRecord, NewAsset, ShapeStatus};
    use photo_domain::{
        AssetId, FileSignature, FolderGroupId, LibraryId, MediaKind, RelativePathKey,
    };
    use photo_indexer::{IndexJob, IndexScheduler, JobPriority, SchedulerConfig};

    use crate::service::SelectionToken;
    use crate::{AppConfig, AppService, DerivativeClass, DerivativeReference, WallUpdate};

    use super::{DerivativeQueue, PendingDerivative};

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
            spec: spec.clone(),
            group: group_id,
            class: DerivativeClass::ScreenPreview,
            selection,
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
            spec: spec.clone(),
            group: group_id,
            class: DerivativeClass::ScreenPreview,
            selection: SelectionToken {
                library_id,
                group_id,
                epoch,
            },
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
            .prefetch_screen_previews(
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
        assert!(cached_wall.is_empty());
        assert_eq!(
            service
                .run_derivative_jobs(pending_wall, JobPriority::Visible)
                .await
                .len(),
            250
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
