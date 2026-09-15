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

use crate::discover::{DiscoveredAsset, SidecarLookup, discover_asset_with_sidecar};
use crate::{IndexError, IndexEvent, ScanProgress, ScanStage, ScanSummary};
use crate::{IndexScheduler, SchedulerConfig};

const INVENTORY_TOTAL_UNKNOWN: u64 = u64::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PhotoInventory {
    direct: u64,
    recursive: u64,
}

struct FolderAdmissionGuard {
    scheduler: Arc<IndexScheduler>,
    folder: Option<(LibraryId, FolderGroupId)>,
}

impl Drop for FolderAdmissionGuard {
    fn drop(&mut self) {
        self.scheduler.finish_folder_enrichment(self.folder);
    }
}

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
        self.start_with_discovery(request, discover_all)
    }

    fn start_with_discovery<F>(
        &self,
        request: ScanRequest,
        discover: F,
    ) -> Result<ScanHandle, IndexError>
    where
        F: FnOnce(
                &Path,
                &Path,
                LibraryId,
                Option<FolderGroupId>,
                mpsc::Sender<DiscoveredAsset>,
                watch::Receiver<bool>,
                &FolderPolicyEngine,
                &mpsc::Sender<IndexEvent>,
                &AtomicU64,
                &AtomicU64,
            ) -> Result<(), IndexError>
            + Send
            + 'static,
    {
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
        let inventory_root = root.clone();
        let discovery_cancel = cancel_rx.clone();
        let inventory_cancel = cancel_rx.clone();
        let library_id = request.library_id;
        let folder_group_id = request.folder_group_id;
        let folder_key = folder_group_id.map(|group| (library_id, group));
        let discovery_failures = Arc::new(AtomicU64::new(0));
        let failed_photos = Arc::new(AtomicU64::new(0));
        let discovery_events = events_tx.clone();
        let discovery_failure_count = discovery_failures.clone();
        let discovery_photo_failures = failed_photos.clone();
        let discovery = tokio::task::spawn_blocking(move || {
            discover(
                &discovery_root,
                &library_root,
                library_id,
                folder_group_id,
                discovered_tx,
                discovery_cancel,
                &policy_engine,
                &discovery_events,
                &discovery_failure_count,
                &discovery_photo_failures,
            )
        });
        let inventory = tokio::task::spawn_blocking(move || {
            count_photo_inventory(&inventory_root, &inventory_cancel)
        });
        let join = tokio::spawn(async move {
            let _admission = FolderAdmissionGuard {
                scheduler: scheduler.clone(),
                folder: folder_key,
            };
            let summary = Arc::new(Mutex::new(ScanSummary::default()));
            let progress = Arc::new((AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)));
            let inventory_total = Arc::new(AtomicU64::new(INVENTORY_TOTAL_UNKNOWN));
            let direct_inventory_total = Arc::new(AtomicU64::new(INVENTORY_TOTAL_UNKNOWN));
            let inventory_publisher = tokio::spawn({
                let events = events_tx.clone();
                let progress = progress.clone();
                let inventory_total = inventory_total.clone();
                let direct_inventory_total = direct_inventory_total.clone();
                async move {
                    match inventory.await {
                        Ok(Ok(Some(inventory))) => {
                            inventory_total.store(inventory.recursive, Ordering::Release);
                            direct_inventory_total.store(inventory.direct, Ordering::Release);
                            let _ = events
                                .send(IndexEvent::Progress(ScanProgress {
                                    stage: ScanStage::Discovering,
                                    discovered: progress.0.load(Ordering::Relaxed),
                                    shaped: progress.1.load(Ordering::Relaxed),
                                    enriched: progress.2.load(Ordering::Relaxed),
                                    direct_total: Some(inventory.direct),
                                    total: Some(inventory.recursive),
                                }))
                                .await;
                        }
                        Ok(Ok(None)) => {}
                        Ok(Err(error)) => {
                            tracing::debug!(%error, "photo inventory count was unavailable");
                        }
                        Err(error) => {
                            tracing::debug!(%error, "photo inventory count task stopped");
                        }
                    }
                }
            });
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
                let inventory_total = inventory_total.clone();
                let direct_inventory_total = direct_inventory_total.clone();
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
                        let counts_as_viewable_photo = item.asset.media_kind.is_wall_viewable();
                        {
                            let mut s = summary.lock().await;
                            s.discovered += 1;
                        }
                        if counts_as_viewable_photo {
                            let discovered = progress.0.fetch_add(1, Ordering::Relaxed) + 1;
                            if discovered.is_multiple_of(32) {
                                let _ = events
                                    .send(IndexEvent::Progress(ScanProgress {
                                        stage: ScanStage::Discovering,
                                        discovered,
                                        shaped: progress.1.load(Ordering::Relaxed),
                                        enriched: progress.2.load(Ordering::Relaxed),
                                        direct_total: known_inventory_total(
                                            &direct_inventory_total,
                                        ),
                                        total: known_inventory_total(&inventory_total),
                                    }))
                                    .await;
                            }
                        }
                        let _ = events
                            .send(IndexEvent::Discovered {
                                asset: item.asset.clone(),
                            })
                            .await;
                        if !admit_enrichment(&cancel, &scheduler, folder_key).await {
                            break;
                        }
                        let path = item.source_path.clone();
                        let shape_result = tokio::task::spawn_blocking(move || {
                            probe_display_shape(&path, MediaProbe::shape)
                        })
                        .await;
                        scheduler.release_folder_enrichment(folder_key);
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
                                if counts_as_viewable_photo {
                                    let shaped = progress.1.fetch_add(1, Ordering::Relaxed) + 1;
                                    if shaped.is_multiple_of(32) {
                                        let _ = events
                                            .send(IndexEvent::Progress(ScanProgress {
                                                stage: ScanStage::Shaping,
                                                discovered: progress.0.load(Ordering::Relaxed),
                                                shaped,
                                                enriched: progress.2.load(Ordering::Relaxed),
                                                direct_total: known_inventory_total(
                                                    &direct_inventory_total,
                                                ),
                                                total: known_inventory_total(&inventory_total),
                                            }))
                                            .await;
                                    }
                                }
                            }
                            Ok(Err(w)) => {
                                let inaccessible = matches!(
                                    w.code,
                                    "source_missing" | "source_unreadable" | "source_check_failed"
                                );
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
                                if counts_as_viewable_photo {
                                    let shaped = progress.1.fetch_add(1, Ordering::Relaxed) + 1;
                                    if shaped.is_multiple_of(32) {
                                        let _ = events
                                            .send(IndexEvent::Progress(ScanProgress {
                                                stage: ScanStage::Shaping,
                                                discovered: progress.0.load(Ordering::Relaxed),
                                                shaped,
                                                enriched: progress.2.load(Ordering::Relaxed),
                                                direct_total: known_inventory_total(
                                                    &direct_inventory_total,
                                                ),
                                                total: known_inventory_total(&inventory_total),
                                            }))
                                            .await;
                                    }
                                }
                                if inaccessible {
                                    continue;
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
                let inventory_total = inventory_total.clone();
                let direct_inventory_total = direct_inventory_total.clone();
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
                        let counts_as_viewable_photo = item.asset.media_kind.is_wall_viewable();
                        if !admit_enrichment(&cancel, &scheduler, folder_key).await {
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
                            scheduler.release_folder_enrichment(folder_key);
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
                                if counts_as_viewable_photo {
                                    let enriched = progress.2.fetch_add(1, Ordering::Relaxed) + 1;
                                    if enriched.is_multiple_of(32) {
                                        let _ = events
                                            .send(IndexEvent::Progress(ScanProgress {
                                                stage: ScanStage::Enriching,
                                                discovered: progress.0.load(Ordering::Relaxed),
                                                shaped: progress.1.load(Ordering::Relaxed),
                                                enriched,
                                                direct_total: known_inventory_total(
                                                    &direct_inventory_total,
                                                ),
                                                total: known_inventory_total(&inventory_total),
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
                        scheduler.release_folder_enrichment(folder_key);
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
            let _ = inventory_publisher.await;
            summary.failed += discovery_failures.load(Ordering::Relaxed);
            summary.discovered += discovery_failures.load(Ordering::Relaxed);
            let discovered =
                progress.0.load(Ordering::Relaxed) + failed_photos.load(Ordering::Relaxed);
            if !summary.cancelled {
                validate_completed_inventory(
                    discovered,
                    failed_photos.load(Ordering::Relaxed),
                    known_inventory(&direct_inventory_total, &inventory_total),
                )?;
            }
            let _ = events_tx
                .send(IndexEvent::Progress(ScanProgress {
                    stage: ScanStage::Completed,
                    discovered,
                    shaped: progress.1.load(Ordering::Relaxed),
                    enriched: progress.2.load(Ordering::Relaxed),
                    direct_total: known_inventory_total(&direct_inventory_total),
                    total: known_inventory_total(&inventory_total).or(Some(discovered)),
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

fn known_inventory_total(total: &AtomicU64) -> Option<u64> {
    match total.load(Ordering::Acquire) {
        INVENTORY_TOTAL_UNKNOWN => None,
        total => Some(total),
    }
}

fn known_inventory(direct: &AtomicU64, recursive: &AtomicU64) -> Option<PhotoInventory> {
    Some(PhotoInventory {
        direct: known_inventory_total(direct)?,
        recursive: known_inventory_total(recursive)?,
    })
}

fn validate_completed_inventory(
    discovered: u64,
    failed_photos: u64,
    inventory: Option<PhotoInventory>,
) -> Result<(), IndexError> {
    // Independent walks may disagree about entries whose discovery metadata
    // failed. Only those observed failures can explain a smaller inventory.
    if let Some(inventory) = inventory
        && (inventory.recursive < discovered.saturating_sub(failed_photos)
            || inventory.recursive > discovered)
    {
        return Err(IndexError::InventoryMismatch {
            expected: inventory.recursive,
            discovered,
        });
    }
    Ok(())
}

fn count_photo_inventory(
    root: &Path,
    cancel: &watch::Receiver<bool>,
) -> Result<Option<PhotoInventory>, IndexError> {
    let mut direct = 0_u64;
    let mut recursive = 0_u64;
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        if *cancel.borrow() {
            return Ok(None);
        }
        let entry = entry.map_err(IndexError::Walk)?;
        if !entry.file_type().is_file() {
            continue;
        }
        if MediaKind::from_path(entry.path()).is_some_and(MediaKind::is_wall_viewable) {
            recursive = recursive.saturating_add(1);
            if entry.depth() == 1 {
                direct = direct.saturating_add(1);
            }
        }
    }
    Ok(Some(PhotoInventory { direct, recursive }))
}

async fn admit_enrichment(
    cancel: &watch::Receiver<bool>,
    scheduler: &IndexScheduler,
    folder: Option<(LibraryId, FolderGroupId)>,
) -> bool {
    loop {
        if *cancel.borrow() {
            return false;
        }
        if scheduler.try_admit_folder_enrichment(folder) {
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
    check_asset_access(&item.source_path)?;
    let colour = MediaProbe::representative_rgb(&item.source_path);
    let metadata = reader.read(&item.source_path, item.sidecar_path.as_deref())?;
    if item.asset.media_kind == MediaKind::Heif
        && let Some(warning) = metadata.warnings.iter().find(|warning| {
            matches!(
                warning.code,
                "source_missing"
                    | "source_unreadable"
                    | "source_check_failed"
                    | "image_open_failed"
                    | "exif_open_failed"
                    | "xmp_open_failed"
                    | "xmp_read_failed"
            )
        })
    {
        return Err(MetadataReadWarning::new(
            warning.code,
            warning.message.clone(),
        ));
    }
    // EXIF readers may return an empty bundle when a file disappears during
    // their read. Keep the catalog's known metadata on a transient access loss.
    if item.asset.media_kind == MediaKind::Heif {
        check_asset_access(&item.source_path)?;
    }
    Ok(ProcessedAsset {
        representative_rgb: colour.as_ref().ok().copied(),
        colour_warning: colour.err(),
        metadata,
    })
}

fn probe_display_shape(
    path: &Path,
    probe: impl FnOnce(&Path) -> Result<photo_metadata::ImageShape, MetadataReadWarning>,
) -> Result<(u32, u32, u16), MetadataReadWarning> {
    check_asset_access(path)?;
    let kind = MediaKind::from_path(path).unwrap_or(MediaKind::Unknown);
    let shape = probe(path).map_err(|warning| {
        // The decoder may flatten an access failure into a shape warning.
        // Distinguish source loss from a terminal corrupt-image outcome.
        if kind == MediaKind::Heif
            && let Err(access) = check_asset_access(path)
        {
            return access;
        }
        warning
    })?;
    let orientation = if kind == MediaKind::Heif {
        1
    } else {
        EmbeddedExifReader::read(path)
            .ok()
            .and_then(|bundle| bundle.orientation)
            .unwrap_or(1)
    };
    let rotated = matches!(orientation, 5..=8);
    Ok((
        if rotated { shape.height } else { shape.width },
        if rotated { shape.width } else { shape.height },
        orientation,
    ))
}

fn check_asset_access(path: &Path) -> Result<(), MetadataReadWarning> {
    std::fs::File::open(path)
        .and_then(|file| file.metadata())
        .map(|_| ())
        .map_err(source_access_warning)
}

fn source_access_warning(error: std::io::Error) -> MetadataReadWarning {
    let code = match error.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => "source_missing",
        std::io::ErrorKind::PermissionDenied => "source_unreadable",
        _ => "source_check_failed",
    };
    MetadataReadWarning::new(code, error.to_string())
}

// Keep this signature aligned with the injectable discovery seam used by the
// deterministic disappearance race test.
#[allow(clippy::too_many_arguments)]
fn discover_all(
    root: &Path,
    library_root: &Path,
    library_id: LibraryId,
    folder_group_id: Option<FolderGroupId>,
    sender: mpsc::Sender<DiscoveredAsset>,
    cancel: watch::Receiver<bool>,
    policy_engine: &FolderPolicyEngine,
    events: &mpsc::Sender<IndexEvent>,
    failures: &AtomicU64,
    failed_photos: &AtomicU64,
) -> Result<(), IndexError> {
    let children = std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .collect::<Vec<_>>();
    let structure = FolderStructureSnapshot::new(children);
    let mut sidecars = SidecarLookup::default();
    for entry in walkdir::WalkDir::new(root).follow_links(false) {
        if *cancel.borrow() {
            break;
        }
        let entry = entry.map_err(IndexError::Walk)?;
        if !entry.file_type().is_file() {
            continue;
        }
        let sidecar = sidecars.find(entry.path())?;
        let Some(mut asset) = discover_for_scan(
            library_root,
            entry.path(),
            library_id,
            sidecar,
            events,
            failures,
            failed_photos,
        )?
        else {
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

fn discover_for_scan(
    root: &Path,
    path: &Path,
    library: LibraryId,
    sidecar: Option<PathBuf>,
    events: &mpsc::Sender<IndexEvent>,
    failures: &AtomicU64,
    failed_photos: &AtomicU64,
) -> Result<Option<DiscoveredAsset>, IndexError> {
    match discover_asset_with_sidecar(root, path, library, sidecar) {
        Err(IndexError::Io(error)) => {
            let relative = path
                .strip_prefix(root)
                .map_err(|_| IndexError::PathOutsideRoot(path.to_owned()))?;
            let relative = photo_domain::RelativePathKey::from_relative_path(relative)
                .map_err(|e| IndexError::InvalidRelativePath(e.to_string()))?;
            let warning = source_access_warning(error);
            let _ = events.blocking_send(IndexEvent::Warning {
                asset_id: Some(photo_domain::AssetId::for_path(library, &relative)),
                code: warning.code,
                message: warning.message,
            });
            failures.fetch_add(1, Ordering::Relaxed);
            if MediaKind::from_path(path).is_some_and(MediaKind::is_wall_viewable) {
                failed_photos.fetch_add(1, Ordering::Relaxed);
            }
            Ok(None)
        }
        result => result,
    }
}

#[cfg(test)]
mod tests {
    use super::{PhotoInventory, validate_completed_inventory};
    use crate::{IndexError, IndexScheduler, InteractionMode, SchedulerConfig};

    #[test]
    fn heif_disappearing_between_access_check_and_shape_probe_keeps_cached_geometry() {
        use crate::{CatalogWriter, IndexEvent};
        use photo_catalog::{Catalog, NewAsset, NewLibrary};
        use photo_domain::{Availability, MediaKind, RelativePathKey};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("portrait.heic");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../codec/tests/fixtures/heif/portrait-rotated.heic"),
            &path,
        )
        .unwrap();
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = catalog
            .add_library(&NewLibrary::configured("Photos", temp.path()))
            .unwrap();
        let generation = catalog.begin_generation(library.id).unwrap();
        let asset = NewAsset::minimal(
            library.id,
            RelativePathKey::from_relative_path(std::path::Path::new("portrait.heic")).unwrap(),
            "portrait.heic",
            MediaKind::Heif,
            path.metadata().unwrap().len(),
        );
        CatalogWriter::new(&mut catalog, library.id, generation)
            .apply_batch(&[
                IndexEvent::Discovered {
                    asset: asset.clone(),
                },
                IndexEvent::ShapeReady {
                    asset_id: asset.id,
                    width: 100,
                    height: 28,
                    orientation: 1,
                },
            ])
            .unwrap();
        let error = super::probe_display_shape(&path, |path| {
            // This dependency is invoked only after the production access check.
            std::fs::rename(path, path.with_extension("away")).unwrap();
            photo_metadata::MediaProbe::shape(path)
        })
        .unwrap_err();
        assert_eq!(
            error.code, "source_missing",
            "lost source was treated as a corrupt image"
        );
        CatalogWriter::new(&mut catalog, library.id, generation)
            .apply_batch(&[
                IndexEvent::ShapeFallback {
                    asset_id: asset.id,
                    width: 4,
                    height: 3,
                    code: error.code,
                    message: error.message.clone(),
                },
                IndexEvent::Warning {
                    asset_id: Some(asset.id),
                    code: error.code,
                    message: error.message,
                },
            ])
            .unwrap();
        let stored = catalog.find_asset(asset.id).unwrap().unwrap();
        assert_eq!((stored.width, stored.height), (Some(100), Some(28)));
        assert_eq!(stored.availability, Availability::Missing);
    }

    #[cfg(unix)]
    #[test]
    fn heif_access_errors_after_the_initial_check_keep_their_source_semantics() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        for expected in ["source_unreadable", "source_check_failed"] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("portrait.heic");
            std::fs::copy(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../codec/tests/fixtures/heif/portrait-rotated.heic"),
                &path,
            )
            .unwrap();
            let error = super::probe_display_shape(&path, |path| {
                if expected == "source_unreadable" {
                    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o000)).unwrap();
                } else {
                    std::fs::rename(path, path.with_extension("away")).unwrap();
                    symlink(path, path).unwrap();
                }
                photo_metadata::MediaProbe::shape(path)
            })
            .unwrap_err();
            assert_eq!(error.code, expected);
        }
    }

    #[test]
    fn accessible_corrupt_heif_shape_remains_a_terminal_shape_warning() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("corrupt.heic");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../codec/tests/fixtures/heif/truncated.heic"),
            &path,
        )
        .unwrap();
        let error =
            super::probe_display_shape(&path, photo_metadata::MediaProbe::shape).unwrap_err();
        assert_eq!(error.code, "shape_read_failed");
    }

    #[test]
    fn missing_jpeg_sidecar_keeps_existing_partial_metadata_behavior() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("photo.jpg");
        image::RgbImage::new(8, 6).save(&path).unwrap();
        let item = super::DiscoveredAsset {
            asset: photo_catalog::NewAsset::minimal(
                photo_domain::LibraryId::new(),
                photo_domain::RelativePathKey::from_relative_path(std::path::Path::new(
                    "photo.jpg",
                ))
                .unwrap(),
                "photo.jpg",
                photo_domain::MediaKind::Jpeg,
                1,
            ),
            source_path: path,
            sidecar_path: Some(temp.path().join("missing.xmp")),
        };
        let processed =
            super::process_asset(std::sync::Arc::new(crate::DefaultMetadataReader), item).unwrap();
        assert!(!processed.metadata.capture_dates.is_empty());
        assert!(
            processed
                .metadata
                .warnings
                .iter()
                .any(|warning| warning.code == "xmp_open_failed")
        );
    }

    #[cfg(all(feature = "heic", unix))]
    #[test]
    fn heif_shape_read_time_io_preserves_geometry_and_prevents_certification() {
        use crate::{CatalogWriter, IndexEvent};
        use photo_catalog::{Catalog, NewAsset, NewFolderGroup, NewLibrary};
        use photo_domain::{FolderGroupId, MediaKind, RelativePathKey};
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("portrait.heic");
        std::fs::copy(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../codec/tests/fixtures/heif/portrait-rotated.heic"),
            &path,
        )
        .unwrap();
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = catalog
            .add_library(&NewLibrary::configured("Photos", temp.path()))
            .unwrap();
        let group = catalog
            .upsert_folder_group(&NewFolderGroup {
                id: FolderGroupId::new(),
                library_id: library.id,
                relative_path: RelativePathKey::from_relative_path(std::path::Path::new(""))
                    .unwrap(),
                display_path: "".into(),
                last_viewed_at: None,
            })
            .unwrap();
        let generation = catalog
            .begin_generation_for_group(library.id, group)
            .unwrap();
        let mut asset = NewAsset::minimal(
            library.id,
            RelativePathKey::from_relative_path(std::path::Path::new("portrait.heic")).unwrap(),
            "portrait.heic",
            MediaKind::Heif,
            path.metadata().unwrap().len(),
        );
        asset.folder_group_id = Some(group);
        CatalogWriter::new(&mut catalog, library.id, generation)
            .apply_batch(&[
                IndexEvent::Discovered {
                    asset: asset.clone(),
                },
                IndexEvent::ShapeReady {
                    asset_id: asset.id,
                    width: 100,
                    height: 28,
                    orientation: 1,
                },
            ])
            .unwrap();
        let error = super::probe_display_shape(&path, |path| {
            std::fs::rename(path, path.with_extension("away")).unwrap();
            std::fs::create_dir(path).unwrap();
            photo_metadata::MediaProbe::shape(path)
        })
        .unwrap_err();
        super::check_asset_access(&path).unwrap();
        assert_eq!(
            error.code, "source_check_failed",
            "read-time typed I/O was flattened despite successful access checks"
        );
        CatalogWriter::new(&mut catalog, library.id, generation)
            .apply_batch(&[
                IndexEvent::ShapeFallback {
                    asset_id: asset.id,
                    width: 4,
                    height: 3,
                    code: error.code,
                    message: error.message.clone(),
                },
                IndexEvent::Warning {
                    asset_id: Some(asset.id),
                    code: error.code,
                    message: error.message,
                },
            ])
            .unwrap();
        catalog
            .complete_generation_for_group(library.id, group, generation)
            .unwrap();
        let stored = catalog.find_asset(asset.id).unwrap().unwrap();
        assert_eq!((stored.width, stored.height), (Some(100), Some(28)));
        assert!(
            catalog
                .heif_metadata_refresh_required(library.id, group)
                .unwrap()
        );
    }

    #[tokio::test]
    async fn disappeared_discovery_joins_and_settles_surviving_assets() {
        use crate::{CatalogWriter, DefaultMetadataReader, IndexEvent, Indexer, ScanRequest};
        use photo_catalog::{Catalog, NewAsset, NewLibrary};
        use photo_domain::{Availability, MediaKind, RelativePathKey};

        let temp = tempfile::tempdir().unwrap();
        image::RgbImage::new(8, 6)
            .save(temp.path().join("survivor.jpg"))
            .unwrap();
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = catalog
            .add_library(&NewLibrary::configured("photos", temp.path()))
            .unwrap();
        let missing = NewAsset::minimal(
            library.id,
            RelativePathKey::from_relative_path(std::path::Path::new("vanished.jpg")).unwrap(),
            "vanished.jpg",
            MediaKind::Jpeg,
            12,
        );
        catalog.upsert_asset(&missing).unwrap();
        let generation = catalog.begin_generation(library.id).unwrap();
        let indexer = Indexer::new(
            DefaultMetadataReader,
            photo_core::FolderPolicyEngine::new(Vec::new()).unwrap(),
        );
        let mut scan = indexer
            .start_with_discovery(
                ScanRequest::new(temp.path()).for_library(library.id),
                |root,
                 library_root,
                 library_id,
                 group,
                 sender,
                 cancel,
                 policy,
                 events,
                 failures,
                 photos| {
                    // Discovery already observed this directory entry; its metadata lookup
                    // now fails. The independent real inventory never sees the entry.
                    assert!(
                        super::discover_for_scan(
                            library_root,
                            &root.join("vanished.jpg"),
                            library_id,
                            None,
                            events,
                            failures,
                            photos
                        )?
                        .is_none()
                    );
                    super::discover_all(
                        root,
                        library_root,
                        library_id,
                        group,
                        sender,
                        cancel,
                        policy,
                        events,
                        failures,
                        photos,
                    )
                },
            )
            .unwrap();
        let mut completed = false;
        while let Some(event) = scan.events.recv().await {
            completed |= matches!(event, IndexEvent::Completed(_));
            CatalogWriter::new(&mut catalog, library.id, generation)
                .apply_batch(&[event])
                .unwrap();
        }
        let summary = scan
            .join()
            .await
            .expect("observed file disappearance must not abort completion");
        assert!(completed);
        assert_eq!(summary.discovered, 2);
        assert_eq!(summary.failed, 1);
        catalog.complete_generation(library.id, generation).unwrap();
        let assets = catalog.assets(library.id).unwrap();
        assert_eq!(assets.len(), 2);
        assert_eq!(
            catalog
                .find_asset(missing.id)
                .unwrap()
                .unwrap()
                .availability,
            Availability::Missing
        );
        assert_eq!(
            assets
                .iter()
                .find(|asset| asset.id != missing.id)
                .unwrap()
                .availability,
            Availability::Available
        );
    }

    #[test]
    fn root_first_disappeared_file_does_not_abort_discovery() {
        let temp = tempfile::tempdir().unwrap();
        let library = photo_domain::LibraryId::new();
        let missing = temp.path().join("gone.jpg");
        let (events, mut received) = tokio::sync::mpsc::channel(4);
        let failures = std::sync::atomic::AtomicU64::new(0);
        let photos = std::sync::atomic::AtomicU64::new(0);
        let result = super::discover_for_scan(
            temp.path(),
            &missing,
            library,
            None,
            &events,
            &failures,
            &photos,
        );
        assert!(
            matches!(result, Ok(None)),
            "a vanished file must be an asset outcome, not a scan error"
        );
        assert!(matches!(
            received.try_recv().unwrap(),
            crate::IndexEvent::Warning {
                asset_id: Some(_),
                code: "source_missing",
                ..
            }
        ));
        assert_eq!(photos.load(std::sync::atomic::Ordering::Relaxed), 1);
        let readable = temp.path().join("next.jpg");
        std::fs::write(&readable, b"photo").unwrap();
        assert!(
            super::discover_for_scan(
                temp.path(),
                &readable,
                library,
                None,
                &events,
                &failures,
                &photos
            )
            .unwrap()
            .is_some()
        );
    }

    #[test]
    fn completion_rejects_a_partial_discovery_against_the_inventory() {
        let error = validate_completed_inventory(
            154,
            0,
            Some(PhotoInventory {
                direct: 2_092,
                recursive: 2_092,
            }),
        )
        .unwrap_err();

        assert!(matches!(
            error,
            IndexError::InventoryMismatch {
                expected: 2_092,
                discovered: 154
            }
        ));
    }

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
