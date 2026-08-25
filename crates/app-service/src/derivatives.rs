//! Derivative request orchestration lives here so source paths never cross the DTO boundary.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use photo_cache::{
    DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget, ImageDerivativeGenerator,
};
use photo_catalog::{Catalog, NewDerivative, WallOrder};
use photo_domain::{AssetId, Availability, DerivativeId, FolderGroupId};
use photo_indexer::{IndexJob, InteractionMode, JobPriority};
use tokio::sync::{Mutex, oneshot};

use crate::{
    AppService, AppServiceError, DerivativeClass, DerivativePriority, DerivativeReference,
    DerivativeRequest, WallUpdate,
};

const DECODER_VERSION: &str = "image-0.25-v1";

pub(crate) struct PendingDerivative {
    id: AssetId,
    source: PathBuf,
    spec: DerivativeSpec,
    group: FolderGroupId,
    class: DerivativeClass,
}

struct QueuedDerivative {
    pending: PendingDerivative,
    result: oneshot::Sender<Option<DerivativeReference>>,
}

#[derive(Default)]
pub(crate) struct DerivativeQueue {
    jobs: Mutex<HashMap<String, QueuedDerivative>>,
    driver: Mutex<()>,
    sequence: AtomicU64,
}

impl DerivativeQueue {
    async fn next_owned(
        &self,
        scheduler: &photo_indexer::IndexScheduler,
    ) -> Option<(IndexJob, QueuedDerivative)> {
        let mut foreign = Vec::new();
        while let Some(job) = scheduler.next().await {
            if let Some(queued) = self.jobs.lock().await.remove(job.name()) {
                for job in foreign {
                    scheduler.enqueue(job).await;
                }
                return Some((job, queued));
            }
            foreign.push(job);
        }
        for job in foreign {
            scheduler.enqueue(job).await;
        }
        None
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
                let has_paused_jobs = !service.derivative_queue.jobs.lock().await.is_empty();
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
                derivatives: Vec::new(),
            });
            return Ok(());
        }

        let (ready, pending) = self.resolve_derivatives(&ids, DerivativeClass::WallThumbnail)?;
        let priority = match request.priority {
            DerivativePriority::Visible => JobPriority::Visible,
            DerivativePriority::NearViewport => JobPriority::NearViewport,
        };
        let generated = self.run_derivative_jobs(pending, priority).await;
        let derivatives = merge_in_request_order(&ids, ready, generated);
        let _ = self
            .updates
            .send(WallUpdate::DerivativesReady { derivatives });

        if let Ok(mut state) = self.state.lock() {
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
    ) -> Result<(Vec<DerivativeReference>, Vec<PendingDerivative>), AppServiceError> {
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
                });
            }
        }
        Ok((ready, pending))
    }

    async fn run_derivative_jobs(
        &self,
        pending: Vec<PendingDerivative>,
        priority: JobPriority,
    ) -> Vec<DerivativeReference> {
        let mut receivers = Vec::with_capacity(pending.len());
        for pending in pending {
            let sequence = self
                .derivative_queue
                .sequence
                .fetch_add(1, Ordering::Relaxed);
            let name = format!(
                "derivative:{sequence}:{}",
                pending.id.as_uuid().hyphenated()
            );
            let (send, receive) = oneshot::channel();
            self.derivative_queue.jobs.lock().await.insert(
                name.clone(),
                QueuedDerivative {
                    pending,
                    result: send,
                },
            );
            self.scheduler.enqueue(IndexJob::new(name, priority)).await;
            receivers.push(receive);
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
            let Some((job, queued)) = self.derivative_queue.next_owned(&self.scheduler).await
            else {
                break;
            };
            if should_pause(
                job.priority(),
                self.scheduler.available_background_permits(),
            ) {
                self.derivative_queue
                    .jobs
                    .lock()
                    .await
                    .insert(job.name().to_owned(), queued);
                self.scheduler.enqueue(job).await;
                break;
            }
            let result = self.generate_derivative(queued.pending).await.ok();
            let _ = queued.result.send(result);
        }
    }

    async fn generate_derivative(
        &self,
        pending: PendingDerivative,
    ) -> Result<DerivativeReference, AppServiceError> {
        let generator = ImageDerivativeGenerator::new(&self.cache_root)
            .map_err(|_| AppServiceError::DerivativeFailed)?;
        let id = pending.id;
        let group = pending.group;
        let class = pending.class;
        let generated = match class {
            DerivativeClass::WallThumbnail => {
                let source = pending.source;
                let spec = pending.spec;
                tokio::task::spawn_blocking(move || generator.generate(&source, &spec))
                    .await
                    .map_err(|_| AppServiceError::DerivativeFailed)?
                    .map_err(|_| AppServiceError::DerivativeFailed)?
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
                .map_err(|_| AppServiceError::DerivativeFailed)?
                .map_err(|_| AppServiceError::DerivativeFailed)?
            }
        };
        if class == DerivativeClass::WallThumbnail {
            let mut state = self.state()?;
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
                })?;
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
        let Ok((cached, pending)) =
            self.resolve_derivatives(&recent, DerivativeClass::ScreenPreview)
        else {
            return;
        };
        let generated = self
            .run_derivative_jobs(pending, JobPriority::NearViewport)
            .await;
        let derivatives = merge_in_request_order(&recent, cached, generated);
        if !derivatives.is_empty() {
            let _ = self
                .updates
                .send(WallUpdate::DerivativesReady { derivatives });
        }

        if self.scheduler.available_background_permits() <= 1 {
            return;
        }
        let remaining = self.remaining_group_ids(&recent).unwrap_or_default();
        if remaining.is_empty() {
            return;
        }
        let Ok((_cached, pending)) =
            self.resolve_derivatives(&remaining, DerivativeClass::ScreenPreview)
        else {
            return;
        };
        let derivatives = self
            .run_derivative_jobs(pending, JobPriority::IdleLibrary)
            .await;
        if !derivatives.is_empty() {
            let _ = self
                .updates
                .send(WallUpdate::DerivativesReady { derivatives });
        }
    }

    fn remaining_group_ids(&self, recent: &[AssetId]) -> Result<Vec<AssetId>, AppServiceError> {
        let recent = recent.iter().copied().collect::<HashSet<_>>();
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
            .has_completed_generation_for_library(selection.library_id)?;
        let order = if settled {
            WallOrder::CapturedAscending
        } else {
            WallOrder::Provisional
        };
        Ok(state
            .libraries
            .catalog()
            .wall_page(group, order, None, 250)?
            .items
            .into_iter()
            .map(|asset| asset.id)
            .filter(|id| !recent.contains(id))
            .collect())
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
    use std::sync::Arc;

    use photo_indexer::{IndexJob, IndexScheduler, JobPriority, SchedulerConfig};

    use super::DerivativeQueue;

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

    #[test]
    fn active_interaction_pauses_only_idle_library_derivatives() {
        assert!(super::should_pause(JobPriority::IdleLibrary, 1));
        assert!(!super::should_pause(JobPriority::NearViewport, 1));
        assert!(!super::should_pause(JobPriority::Visible, 1));
        assert!(!super::should_pause(JobPriority::IdleLibrary, 4));
    }
}
