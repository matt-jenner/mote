use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use photo_core::{FolderPolicyEngine, FolderStructureSnapshot};
use photo_domain::{FolderGroupId, LibraryId, MediaKind};
use photo_metadata::{
    EmbeddedExifReader, MediaProbe, MetadataBundle, MetadataReadWarning, MetadataResolver,
    RepresentativeRgb,
};
use tokio::sync::{Mutex, mpsc, watch};

use crate::discover::{DiscoveredAsset, discover_asset};
use crate::{IndexError, IndexEvent, ScanProgress, ScanStage, ScanSummary};
use crate::{IndexScheduler, SchedulerConfig};

pub trait MetadataReader: Send + Sync + 'static {
    fn read(
        &self,
        media_path: &Path,
        sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning>;
}

pub struct Indexer<R> {
    reader: Arc<R>,
    policy_engine: Arc<FolderPolicyEngine>,
    scheduler: Arc<IndexScheduler>,
}

pub struct ScanRequest {
    pub root: PathBuf,
    pub library_id: LibraryId,
    pub library_root: Option<PathBuf>,
    pub selection_root: Option<PathBuf>,
    pub folder_group_id: Option<FolderGroupId>,
}

impl ScanRequest {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            library_id: LibraryId::new(),
            library_root: None,
            selection_root: None,
            folder_group_id: None,
        }
    }

    pub fn for_library(mut self, library_id: LibraryId) -> Self {
        self.library_id = library_id;
        self
    }

    pub fn roots(
        mut self,
        library_root: impl Into<PathBuf>,
        selection_root: impl Into<PathBuf>,
    ) -> Self {
        self.library_root = Some(library_root.into());
        self.selection_root = Some(selection_root.into());
        self
    }
    pub fn for_folder_group(mut self, folder_group_id: FolderGroupId) -> Self {
        self.folder_group_id = Some(folder_group_id);
        self
    }
}

pub struct ScanHandle {
    pub events: mpsc::Receiver<IndexEvent>,
    cancel: watch::Sender<bool>,
    join: tokio::task::JoinHandle<Result<ScanSummary, IndexError>>,
}

impl ScanHandle {
    pub fn cancellation_sender(&self) -> watch::Sender<bool> {
        self.cancel.clone()
    }
    pub fn cancel(&self) -> Result<(), IndexError> {
        self.cancel
            .send(true)
            .map_err(|_| IndexError::CancellationClosed)
    }

    pub async fn join(self) -> Result<ScanSummary, IndexError> {
        let Self {
            events,
            cancel: _,
            join,
        } = self;
        drop(events);
        join.await
            .map_err(|error| IndexError::TaskJoin(error.to_string()))?
    }
}

impl<R: MetadataReader> Indexer<R> {
    pub fn new(reader: R, policy_engine: FolderPolicyEngine) -> Self {
        Self::with_scheduler(
            reader,
            policy_engine,
            Arc::new(IndexScheduler::new(SchedulerConfig::default())),
        )
    }

    pub fn with_scheduler(
        reader: R,
        policy_engine: FolderPolicyEngine,
        scheduler: Arc<IndexScheduler>,
    ) -> Self {
        Self {
            reader: Arc::new(reader),
            policy_engine: Arc::new(policy_engine),
            scheduler,
        }
    }

    pub fn start(&self, request: ScanRequest) -> Result<ScanHandle, IndexError> {
        if !request.root.is_dir() {
            return Err(IndexError::RootUnavailable(request.root));
        }
        let root = std::fs::canonicalize(
            request
                .selection_root
                .clone()
                .unwrap_or(request.root.clone()),
        )?;
        let library_root =
            std::fs::canonicalize(request.library_root.unwrap_or_else(|| root.clone()))?;
        let (events_tx, events) = mpsc::channel(128);
        let (discovered_tx, discovered_rx) = mpsc::channel(64);
        let (cancel, cancel_rx) = watch::channel(false);
        let reader = self.reader.clone();
        let policy_engine = self.policy_engine.clone();
        let enrichment_workers = self.scheduler.available_background_permits().clamp(1, 2);
        let scheduler = self.scheduler.clone();
        let discovery_root = root.clone();
        let discovery_cancel = cancel_rx.clone();
        let library_id = request.library_id;
        let folder_group_id = request.folder_group_id;
        let discovery = tokio::task::spawn_blocking(move || {
            discover_all(
                &discovery_root,
                &library_root,
                library_id,
                folder_group_id,
                discovered_tx,
                discovery_cancel,
                &policy_engine,
            )
        });
        let join = tokio::spawn(async move {
            let summary = Arc::new(Mutex::new(ScanSummary::default()));
            let progress = Arc::new((AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)));
            let (enrich_tx, enrich_rx) = mpsc::channel::<DiscoveredAsset>(64);
            let shared_discovered = Arc::new(Mutex::new(discovered_rx));
            let shared_enrich = Arc::new(Mutex::new(enrich_rx));
            let mut workers = Vec::new();
            for _ in 0..4 {
                let rx = shared_discovered.clone();
                let tx = enrich_tx.clone();
                let events = events_tx.clone();
                let mut cancel = cancel_rx.clone();
                let summary = summary.clone();
                let progress = progress.clone();
                let scheduler = scheduler.clone();
                workers.push(tokio::spawn(async move {
                    loop {
                        let item = tokio::select! {
                            item = async { rx.lock().await.recv().await } => item,
                            _ = cancel.changed() => None,
                        };
                        let Some(item) = item else { break };
                        if *cancel.borrow() {
                            break;
                        }
                        let asset_id = item.asset.id;
                        let counts_as_photo = item.asset.media_kind != MediaKind::Video;
                        {
                            let mut s = summary.lock().await;
                            s.discovered += 1;
                        }
                        if counts_as_photo {
                            let discovered = progress.0.fetch_add(1, Ordering::Relaxed) + 1;
                            if discovered.is_multiple_of(32) {
                                let _ = events
                                    .send(IndexEvent::Progress(ScanProgress {
                                        stage: ScanStage::Discovering,
                                        discovered,
                                        shaped: progress.1.load(Ordering::Relaxed),
                                        enriched: progress.2.load(Ordering::Relaxed),
                                        total: None,
                                    }))
                                    .await;
                            }
                        }
                        let _ = events
                            .send(IndexEvent::Discovered {
                                asset: item.asset.clone(),
                            })
                            .await;
                        if !admit_enrichment(&cancel, &scheduler).await {
                            break;
                        }
                        let path = item.source_path.clone();
                        let shape_result = tokio::task::spawn_blocking(move || {
                            let shape = MediaProbe::shape(&path)?;
                            let orientation = EmbeddedExifReader::read(&path)
                                .ok()
                                .and_then(|bundle| bundle.orientation)
                                .unwrap_or(1);
                            let rotated = matches!(orientation, 5..=8);
                            Ok::<_, MetadataReadWarning>((
                                if rotated { shape.height } else { shape.width },
                                if rotated { shape.width } else { shape.height },
                                orientation,
                            ))
                        })
                        .await;
                        scheduler.release_enrichment();
                        match shape_result {
                            Ok(Ok((width, height, orientation))) => {
                                let _ = events
                                    .send(IndexEvent::ShapeReady {
                                        asset_id,
                                        width,
                                        height,
                                        orientation,
                                    })
                                    .await;
                                if counts_as_photo {
                                    let shaped = progress.1.fetch_add(1, Ordering::Relaxed) + 1;
                                    if shaped.is_multiple_of(32) {
                                        let _ = events
                                            .send(IndexEvent::Progress(ScanProgress {
                                                stage: ScanStage::Shaping,
                                                discovered: progress.0.load(Ordering::Relaxed),
                                                shaped,
                                                enriched: progress.2.load(Ordering::Relaxed),
                                                total: None,
                                            }))
                                            .await;
                                    }
                                }
                            }
                            Ok(Err(w)) => {
                                summary.lock().await.failed += 1;
                                let _ = events
                                    .send(IndexEvent::ShapeFallback {
                                        asset_id,
                                        width: 4,
                                        height: 3,
                                        code: w.code,
                                        message: w.message.clone(),
                                    })
                                    .await;
                                let _ = events
                                    .send(IndexEvent::Warning {
                                        asset_id: Some(asset_id),
                                        code: w.code,
                                        message: w.message,
                                    })
                                    .await;
                                if counts_as_photo {
                                    let shaped = progress.1.fetch_add(1, Ordering::Relaxed) + 1;
                                    if shaped.is_multiple_of(32) {
                                        let _ = events
                                            .send(IndexEvent::Progress(ScanProgress {
                                                stage: ScanStage::Shaping,
                                                discovered: progress.0.load(Ordering::Relaxed),
                                                shaped,
                                                enriched: progress.2.load(Ordering::Relaxed),
                                                total: None,
                                            }))
                                            .await;
                                    }
                                }
                            }
                            Err(_) => {}
                        }
                        tokio::select! {
                            result = tx.send(item) => if result.is_err() { break },
                            _ = cancel.changed() => break,
                        }
                    }
                }));
            }
            drop(enrich_tx);
            for _ in 0..enrichment_workers {
                let rx = shared_enrich.clone();
                let events = events_tx.clone();
                let reader = reader.clone();
                let mut cancel = cancel_rx.clone();
                let summary = summary.clone();
                let progress = progress.clone();
                let scheduler = scheduler.clone();
                workers.push(tokio::spawn(async move {
                    loop {
                        let item = tokio::select! {
                            item = async { rx.lock().await.recv().await } => item,
                            _ = cancel.changed() => None,
                        };
                        let Some(item) = item else { break };
                        if *cancel.borrow() {
                            break;
                        }
                        let id = item.asset.id;
                        let counts_as_photo = item.asset.media_kind != MediaKind::Video;
                        if !admit_enrichment(&cancel, &scheduler).await {
                            break;
                        }
                        let reader = reader.clone();
                        let process =
                            tokio::task::spawn_blocking(move || process_asset(reader, item));
                        tokio::pin!(process);
                        let result = tokio::select! {
                            result = &mut process => Some(result),
                            _ = cancel.changed() => {
                                // A blocking metadata read cannot be aborted by
                                // dropping its JoinHandle. Wait for the admitted
                                // work to quiesce so a cancelled recovery cannot
                                // overlap its replacement scan.
                                let _ = process.await;
                                None
                            },
                        };
                        let Some(result) = result else {
                            scheduler.release_enrichment();
                            break;
                        };
                        match result {
                            Ok(Ok(p)) => {
                                for warning in &p.metadata.warnings {
                                    let _ = events
                                        .send(IndexEvent::Warning {
                                            asset_id: Some(id),
                                            code: warning.code,
                                            message: warning.message.clone(),
                                        })
                                        .await;
                                }
                                if let Some(rgb) = p.representative_rgb {
                                    let _ = events
                                        .send(IndexEvent::ColourReady {
                                            asset_id: id,
                                            representative_rgb: rgb,
                                        })
                                        .await;
                                }
                                if let Some(w) = p.colour_warning {
                                    let _ = events
                                        .send(IndexEvent::Warning {
                                            asset_id: Some(id),
                                            code: w.code,
                                            message: w.message,
                                        })
                                        .await;
                                }
                                let _ = events
                                    .send(IndexEvent::MetadataReady {
                                        asset_id: id,
                                        metadata: MetadataResolver::resolve(p.metadata),
                                    })
                                    .await;
                                if counts_as_photo {
                                    let enriched = progress.2.fetch_add(1, Ordering::Relaxed) + 1;
                                    if enriched.is_multiple_of(32) {
                                        let _ = events
                                            .send(IndexEvent::Progress(ScanProgress {
                                                stage: ScanStage::Enriching,
                                                discovered: progress.0.load(Ordering::Relaxed),
                                                shaped: progress.1.load(Ordering::Relaxed),
                                                enriched,
                                                total: None,
                                            }))
                                            .await;
                                    }
                                }
                            }
                            Ok(Err(w)) => {
                                summary.lock().await.failed += 1;
                                let _ = events
                                    .send(IndexEvent::Warning {
                                        asset_id: Some(id),
                                        code: w.code,
                                        message: w.message,
                                    })
                                    .await;
                            }
                            Err(_) => {}
                        }
                        scheduler.release_enrichment();
                    }
                }));
            }
            for worker in workers {
                let _ = worker.await;
            }
            let mut summary = *summary.lock().await;
            if *cancel_rx.borrow() {
                summary.cancelled = true;
            }
            match discovery.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if summary.cancelled => {
                    tracing::debug!(%error, "discovery stopped after cancellation");
                }
                Ok(Err(error)) => return Err(error),
                Err(error) => return Err(IndexError::TaskJoin(error.to_string())),
            }
            let _ = events_tx
                .send(IndexEvent::Progress(ScanProgress {
                    stage: ScanStage::Completed,
                    discovered: progress.0.load(Ordering::Relaxed),
                    shaped: progress.1.load(Ordering::Relaxed),
                    enriched: progress.2.load(Ordering::Relaxed),
                    total: Some(progress.0.load(Ordering::Relaxed)),
                }))
                .await;
            let _ = events_tx.send(IndexEvent::Completed(summary)).await;
            Ok(summary)
        });
        Ok(ScanHandle {
            events,
            cancel,
            join,
        })
    }
}

async fn admit_enrichment(cancel: &watch::Receiver<bool>, scheduler: &IndexScheduler) -> bool {
    loop {
        if *cancel.borrow() {
            return false;
        }
        if scheduler.try_admit_enrichment() {
            return true;
        }
        tokio::task::yield_now().await;
    }
}

struct ProcessedAsset {
    representative_rgb: Option<RepresentativeRgb>,
    colour_warning: Option<MetadataReadWarning>,
    metadata: MetadataBundle,
}

fn process_asset<R: MetadataReader>(
    reader: Arc<R>,
    item: DiscoveredAsset,
) -> Result<ProcessedAsset, MetadataReadWarning> {
    let colour = MediaProbe::representative_rgb(&item.source_path);
    let metadata = reader.read(&item.source_path, item.sidecar_path.as_deref())?;
    Ok(ProcessedAsset {
        representative_rgb: colour.as_ref().ok().copied(),
        colour_warning: colour.err(),
        metadata,
    })
}

fn discover_all(
    root: &Path,
    library_root: &Path,
    library_id: LibraryId,
    folder_group_id: Option<FolderGroupId>,
    sender: mpsc::Sender<DiscoveredAsset>,
    cancel: watch::Receiver<bool>,
    policy_engine: &FolderPolicyEngine,
) -> Result<(), IndexError> {
    let children = std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    let structure = FolderStructureSnapshot::new(children);
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        if *cancel.borrow() {
            break;
        }
        let entry = entry.map_err(IndexError::Walk)?;
        if !entry.file_type().is_file() {
            continue;
        }
        let Some(mut asset) = discover_asset(library_root, entry.path(), library_id)? else {
            continue;
        };
        asset.asset.folder_group_id = folder_group_id;
        let relative = asset
            .source_path
            .strip_prefix(root)
            .map_err(|_| IndexError::PathOutsideRoot(asset.source_path.clone()))?;
        policy_engine
            .classify(relative, &structure)
            .map_err(|error| IndexError::Policy(error.to_string()))?;
        let mut pending = asset;
        loop {
            if *cancel.borrow() {
                return Ok(());
            }
            match sender.try_send(pending) {
                Ok(()) => break,
                Err(mpsc::error::TrySendError::Full(asset)) => {
                    pending = asset;
                    std::thread::park_timeout(std::time::Duration::from_millis(2));
                }
                Err(mpsc::error::TrySendError::Closed(_)) => return Ok(()),
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::{IndexScheduler, InteractionMode, SchedulerConfig};

    #[tokio::test]
    async fn admission_decision_respects_active_limit() {
        let scheduler = IndexScheduler::new(SchedulerConfig {
            idle_workers: 2,
            active_workers: 1,
        });
        scheduler
            .set_interaction_mode(InteractionMode::Active)
            .await;
        assert!(scheduler.try_admit_enrichment());
        assert!(!scheduler.try_admit_enrichment());
        scheduler.release_enrichment();
    }
}
