use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[cfg(feature = "heic")]
use photo_cache::{DerivativeKind, DerivativeSpec, DerivativeTarget, ImageDerivativeGenerator};
use photo_catalog::{Catalog, NewAsset, NewFolderGroup, NewLibrary, ShapeStatus, WallOrder};
use photo_core::FolderPolicyEngine;
use photo_domain::{FolderGroupId, GalleryScope, MediaKind, RelativePathKey};
use photo_indexer::{
    CatalogWriter, DefaultMetadataReader, IndexEvent, IndexScheduler, Indexer, InteractionMode,
    MetadataReader, ScanProgress, ScanRequest, ScanStage, SchedulerConfig,
};
use photo_metadata::{
    Keyword, MetadataBundle, MetadataReadWarning, ProvenanceRecord, RepresentativeRgb,
    ResolvedMetadata,
};

#[derive(Clone, Default)]
struct NoopMetadataReader;

#[cfg(unix)]
#[tokio::test]
async fn root_first_unreadable_file_is_an_asset_warning_and_scan_continues() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("unreadable.jpg");
    std::fs::write(&path, b"unreadable").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).unwrap();
    std::fs::write(temp.path().join("readable.jpg"), b"invalid image data").unwrap();
    let indexer = Indexer::new(
        NoopMetadataReader,
        FolderPolicyEngine::new(Vec::new()).unwrap(),
    );
    let mut handle = indexer.start(ScanRequest::new(temp.path())).unwrap();
    let mut warnings = Vec::new();
    while let Some(event) = handle.events.recv().await {
        if let IndexEvent::Warning { asset_id, code, .. } = event {
            warnings.push((asset_id, code));
        }
    }
    let summary = handle.join().await.unwrap();
    assert_eq!(summary.discovered, 2);
    assert!(
        warnings
            .iter()
            .any(|(asset, code)| asset.is_some() && *code == "source_unreadable")
    );
    assert_eq!(
        warnings
            .iter()
            .filter(|(_, code)| *code == "source_unreadable")
            .count(),
        1
    );
}

#[test]
fn root_first_source_warning_updates_only_the_current_asset_generation() {
    use photo_domain::Availability;
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("photos", Path::new("/photos")))
        .unwrap();
    let generation = catalog.begin_generation(library.id).unwrap();
    let asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("one.jpg")).unwrap(),
        "one.jpg",
        MediaKind::Jpeg,
        12,
    );
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&[
            IndexEvent::Discovered {
                asset: asset.clone(),
            },
            IndexEvent::Warning {
                asset_id: Some(asset.id),
                code: "source_unreadable",
                message: "Cannot read source photo".into(),
            },
        ])
        .unwrap();
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().availability,
        Availability::Unreadable
    );
    catalog.complete_generation(library.id, generation).unwrap();
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().availability,
        Availability::Unreadable,
        "scan completion must preserve confirmed file access failures"
    );
    let next = catalog.begin_generation(library.id).unwrap();
    CatalogWriter::new(&mut catalog, library.id, next)
        .apply_batch(&[IndexEvent::Discovered {
            asset: asset.clone(),
        }])
        .unwrap();
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&[IndexEvent::Warning {
            asset_id: Some(asset.id),
            code: "source_missing",
            message: "Source photo is missing".into(),
        }])
        .unwrap();
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().availability,
        Availability::Available,
        "old access events must not downgrade a newer observation"
    );
    CatalogWriter::new(&mut catalog, library.id, next)
        .apply_batch(&[IndexEvent::Warning {
            asset_id: Some(asset.id),
            code: "shape_failed",
            message: "Cannot decode photo".into(),
        }])
        .unwrap();
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().availability,
        Availability::Available,
        "decode failures are not access failures"
    );
    catalog.complete_generation(library.id, next).unwrap();
    let third = catalog.begin_generation(library.id).unwrap();
    CatalogWriter::new(&mut catalog, library.id, third)
        .apply_batch(&[IndexEvent::Warning {
            asset_id: Some(asset.id),
            code: "source_unreadable",
            message: "Source metadata cannot be read".into(),
        }])
        .unwrap();
    catalog.complete_generation(library.id, third).unwrap();
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().availability,
        Availability::Unreadable,
        "an inaccessible cached file was observed even without a new metadata record"
    );
}

#[test]
fn root_first_indeterminate_access_to_an_uncatalogued_file_does_not_abort_the_batch() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("photos", Path::new("/photos")))
        .unwrap();
    let generation = catalog.begin_generation(library.id).unwrap();
    let asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("good.jpg")).unwrap(),
        "good.jpg",
        MediaKind::Jpeg,
        12,
    );
    let unknown = photo_domain::AssetId::for_path(
        library.id,
        &RelativePathKey::from_relative_path(Path::new("unknown.jpg")).unwrap(),
    );
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&[
            IndexEvent::Warning {
                asset_id: Some(unknown),
                code: "source_check_failed",
                message: "File metadata check failed".into(),
            },
            IndexEvent::Discovered {
                asset: asset.clone(),
            },
        ])
        .expect("one indeterminate file must not roll back siblings");
    assert!(catalog.find_asset(asset.id).unwrap().is_some());
    catalog.complete_generation(library.id, generation).unwrap();
    let next = catalog.begin_generation(library.id).unwrap();
    CatalogWriter::new(&mut catalog, library.id, next)
        .apply_batch(&[IndexEvent::Warning {
            asset_id: Some(asset.id),
            code: "source_check_failed",
            message: "File metadata check failed".into(),
        }])
        .unwrap();
    catalog.complete_generation(library.id, next).unwrap();
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().availability,
        photo_domain::Availability::Available,
        "indeterminate checks must preserve cached availability"
    );
}

impl MetadataReader for NoopMetadataReader {
    fn read(
        &self,
        _media_path: &Path,
        _sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        Ok(MetadataBundle::default())
    }
}

#[derive(Clone)]
struct BlockingMetadataReader {
    gate: Arc<(Mutex<bool>, Condvar)>,
}

#[derive(Clone, Default)]
struct CountingMetadataReader {
    reads: Arc<AtomicUsize>,
}

impl MetadataReader for CountingMetadataReader {
    fn read(
        &self,
        _media_path: &Path,
        _sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        Ok(MetadataBundle::default())
    }
}

impl BlockingMetadataReader {
    fn new() -> (Self, ReleaseMetadata) {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        (Self { gate: gate.clone() }, ReleaseMetadata { gate })
    }
}

impl MetadataReader for BlockingMetadataReader {
    fn read(
        &self,
        _media_path: &Path,
        _sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        let (lock, changed) = &*self.gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = changed.wait(released).unwrap();
        }
        Ok(MetadataBundle::default())
    }
}

struct ReleaseMetadata {
    gate: Arc<(Mutex<bool>, Condvar)>,
}

#[derive(Clone)]
struct AdmissionReader {
    starts: Arc<AtomicUsize>,
    started: std::sync::mpsc::Sender<()>,
    gate: Arc<(Mutex<bool>, Condvar)>,
}

#[derive(Clone)]
struct PermitMetadataReader {
    starts: Arc<AtomicUsize>,
    started: std::sync::mpsc::Sender<()>,
    gate: Arc<(Mutex<usize>, Condvar)>,
}

struct PermitRelease {
    gate: Arc<(Mutex<usize>, Condvar)>,
}

impl AdmissionReader {
    fn new() -> (Self, std::sync::mpsc::Receiver<()>, ReleaseMetadata) {
        let (started, notifications) = std::sync::mpsc::channel();
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        (
            Self {
                starts: Arc::new(AtomicUsize::new(0)),
                started,
                gate: gate.clone(),
            },
            notifications,
            ReleaseMetadata { gate },
        )
    }
}

impl PermitMetadataReader {
    fn new() -> (Self, std::sync::mpsc::Receiver<()>, PermitRelease) {
        let (started, notifications) = std::sync::mpsc::channel();
        let gate = Arc::new((Mutex::new(0), Condvar::new()));
        (
            Self {
                starts: Arc::new(AtomicUsize::new(0)),
                started,
                gate: gate.clone(),
            },
            notifications,
            PermitRelease { gate },
        )
    }
}

impl MetadataReader for AdmissionReader {
    fn read(
        &self,
        _media_path: &Path,
        _sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.started.send(()).unwrap();
        let (lock, changed) = &*self.gate;
        let mut released = lock.lock().unwrap();
        while !*released {
            released = changed.wait(released).unwrap();
        }
        Ok(MetadataBundle::default())
    }
}

impl MetadataReader for PermitMetadataReader {
    fn read(
        &self,
        _media_path: &Path,
        _sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.started.send(()).unwrap();
        let (lock, changed) = &*self.gate;
        let mut permits = lock.lock().unwrap();
        while *permits == 0 {
            permits = changed.wait(permits).unwrap();
        }
        *permits -= 1;
        Ok(MetadataBundle::default())
    }
}

async fn wait_for_starts(
    notifications: std::sync::mpsc::Receiver<()>,
    count: usize,
) -> std::sync::mpsc::Receiver<()> {
    tokio::time::timeout(
        Duration::from_secs(1),
        tokio::task::spawn_blocking(move || {
            for _ in 0..count {
                notifications.recv().unwrap();
            }
            notifications
        }),
    )
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
async fn enrichment_admission_tracks_shared_scheduler_interaction_mode() {
    let fixture = tempfile::tempdir().unwrap();
    write_png(&fixture.path().join("a.png"), [255, 0, 0]);
    write_png(&fixture.path().join("b.png"), [0, 0, 255]);
    let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig {
        idle_workers: 4,
        active_workers: 1,
    }));
    let (reader, notifications, release) = AdmissionReader::new();
    let indexer = Indexer::with_scheduler(reader.clone(), empty_policy_engine(), scheduler.clone());

    let scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let _notifications = wait_for_starts(notifications, 2).await;
    assert_eq!(reader.starts.load(Ordering::SeqCst), 2);
    release.release();
    scan.join().await.unwrap();

    scheduler
        .set_interaction_mode(InteractionMode::Active)
        .await;
    let (reader, notifications, release) = AdmissionReader::new();
    let indexer = Indexer::with_scheduler(reader.clone(), empty_policy_engine(), scheduler.clone());
    let scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let notifications = wait_for_starts(notifications, 1).await;
    assert_eq!(reader.starts.load(Ordering::SeqCst), 1);
    assert!(notifications.try_recv().is_err());
    release.release();
    scan.join().await.unwrap();

    scheduler.set_interaction_mode(InteractionMode::Idle).await;
    let (reader, notifications, release) = AdmissionReader::new();
    let indexer = Indexer::with_scheduler(reader.clone(), empty_policy_engine(), scheduler);
    let scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let _notifications = wait_for_starts(notifications, 2).await;
    assert_eq!(reader.starts.load(Ordering::SeqCst), 2);
    release.release();
    scan.join().await.unwrap();
}

#[tokio::test]
async fn enrichment_admission_tracks_mode_changes_during_one_scan() {
    let fixture = tempfile::tempdir().unwrap();
    for index in 0..8 {
        write_png(&fixture.path().join(format!("{index:02}.png")), [1, 2, 3]);
    }
    let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig {
        idle_workers: 2,
        active_workers: 1,
    }));
    let (reader, notifications, release) = PermitMetadataReader::new();
    let indexer = Indexer::with_scheduler(reader.clone(), empty_policy_engine(), scheduler.clone());

    let scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let notifications = wait_for_starts(notifications, 2).await;
    scheduler
        .set_interaction_mode(InteractionMode::Active)
        .await;
    // Both idle admissions are held, so the mode change is observed before either read completes.
    release.release(2);
    let notifications = wait_for_starts(notifications, 1).await;
    let active_starts = reader.starts.load(Ordering::SeqCst);

    scheduler.set_interaction_mode(InteractionMode::Idle).await;
    release.release(1);
    let _notifications = wait_for_starts(notifications, 2).await;
    let idle_starts = reader.starts.load(Ordering::SeqCst);

    scan.cancel().unwrap();
    release.open();
    scan.join().await.unwrap();
    assert_eq!(active_starts, 3);
    assert_eq!(idle_starts, 5);
}

impl ReleaseMetadata {
    fn release(self) {
        let (lock, changed) = &*self.gate;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}

impl PermitRelease {
    fn release(&self, count: usize) {
        let (lock, changed) = &*self.gate;
        *lock.lock().unwrap() += count;
        changed.notify_all();
    }

    fn open(&self) {
        let (lock, changed) = &*self.gate;
        *lock.lock().unwrap() = usize::MAX;
        changed.notify_all();
    }
}

impl Drop for PermitRelease {
    fn drop(&mut self) {
        self.open();
    }
}

fn write_png(path: &Path, rgb: [u8; 3]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    image::RgbImage::from_raw(1, 1, rgb.to_vec())
        .unwrap()
        .save(path)
        .unwrap();
}

fn empty_policy_engine() -> FolderPolicyEngine {
    FolderPolicyEngine::new(Vec::new()).unwrap()
}

#[tokio::test]
async fn emits_shape_before_blocked_metadata_and_before_scan_completion() {
    let fixture = tempfile::tempdir().unwrap();
    write_png(&fixture.path().join("a.png"), [255, 0, 0]);
    write_png(&fixture.path().join("b.png"), [0, 0, 255]);
    let (reader, release) = BlockingMetadataReader::new();
    let indexer = Indexer::new(reader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();

    let shaped = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(IndexEvent::ShapeReady { width, height, .. }) = scan.events.recv().await {
                break (width, height);
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(shaped, (1, 1));
    release.release();
    let summary = scan.join().await.unwrap();
    assert_eq!(summary.discovered, 2);
    assert_eq!(summary.failed, 0);
}

#[tokio::test]
async fn inventory_total_arrives_while_image_reads_are_paused() {
    let blocker = tempfile::tempdir().unwrap();
    write_png(&blocker.path().join("blocker.png"), [9, 8, 7]);
    let fixture = tempfile::tempdir().unwrap();
    for index in 0..160 {
        write_png(&fixture.path().join(format!("{index:04}.png")), [1, 2, 3]);
    }
    std::fs::write(fixture.path().join("clip.mp4"), b"not a video").unwrap();
    let scheduler = Arc::new(IndexScheduler::new(SchedulerConfig {
        idle_workers: 1,
        active_workers: 1,
    }));
    let (blocking_reader, notifications, release) = AdmissionReader::new();
    let blocking_scan =
        Indexer::with_scheduler(blocking_reader, empty_policy_engine(), scheduler.clone())
            .start(ScanRequest::new(blocker.path()))
            .unwrap();
    let _notifications = wait_for_starts(notifications, 1).await;
    let indexer = Indexer::with_scheduler(NoopMetadataReader, empty_policy_engine(), scheduler);
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();

    let total = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(IndexEvent::Progress(progress)) = scan.events.recv().await
                && progress.total.is_some()
            {
                break (progress.total, progress.shaped);
            }
        }
    })
    .await;
    let _ = scan.cancel();
    release.release();
    scan.join().await.unwrap();
    blocking_scan.join().await.unwrap();

    assert_eq!(total.unwrap(), (Some(160), 0));
}

#[tokio::test]
async fn inventory_reports_direct_and_recursive_photo_totals_in_one_pass() {
    let fixture = tempfile::tempdir().unwrap();
    write_png(&fixture.path().join("direct-a.jpg"), [1, 2, 3]);
    write_png(&fixture.path().join("direct-b.jpg"), [4, 5, 6]);
    write_png(&fixture.path().join("nested/child-a.jpg"), [7, 8, 9]);
    write_png(&fixture.path().join("nested/child-b.jpg"), [10, 11, 12]);
    write_png(
        &fixture.path().join("nested/deeper/child-c.jpg"),
        [13, 14, 15],
    );
    std::fs::write(fixture.path().join("nested/clip.mp4"), b"not a video").unwrap();

    let indexer = Indexer::new(NoopMetadataReader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let totals = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(IndexEvent::Progress(progress)) = scan.events.recv().await
                && progress.total.is_some()
            {
                break (progress.direct_total, progress.total);
            }
        }
    })
    .await
    .unwrap();
    scan.join().await.unwrap();

    assert_eq!(totals, (Some(2), Some(5)));
}

#[tokio::test]
async fn inventory_total_counts_only_formats_the_wall_can_display() {
    let fixture = tempfile::tempdir().unwrap();
    write_png(&fixture.path().join("displayable.jpg"), [1, 2, 3]);
    std::fs::write(fixture.path().join("camera-raw.dng"), b"raw fixture").unwrap();
    std::fs::write(fixture.path().join("phone-photo.heic"), b"heif fixture").unwrap();

    let indexer = Indexer::new(NoopMetadataReader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let (inventory_total, completed) = tokio::time::timeout(Duration::from_secs(1), async {
        let mut inventory_total = None;
        loop {
            if let Some(IndexEvent::Progress(progress)) = scan.events.recv().await {
                inventory_total = inventory_total.or(progress.total);
                if progress.stage == ScanStage::Completed {
                    break (inventory_total, progress);
                }
            }
        }
    })
    .await
    .unwrap();
    let summary = scan.join().await.unwrap();

    assert_eq!(inventory_total, Some(1));
    assert_eq!(completed.direct_total, Some(1));
    assert_eq!(completed.total, Some(1));
    assert_eq!(completed.discovered, 1);
    assert_eq!(completed.shaped, 1);
    assert_eq!(completed.enriched, 1);
    assert_eq!(summary.discovered, 3);
}

#[tokio::test]
async fn video_discovery_is_indexed_but_not_counted_in_photo_progress() {
    let fixture = tempfile::tempdir().unwrap();
    write_png(&fixture.path().join("a.jpg"), [255, 0, 0]);
    write_png(&fixture.path().join("b.jpg"), [0, 0, 255]);
    std::fs::write(fixture.path().join("clip.mp4"), b"not a video").unwrap();

    let indexer = Indexer::new(NoopMetadataReader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let mut discovered = Vec::new();
    let mut final_progress = None;
    while let Some(event) = scan.events.recv().await {
        match event {
            IndexEvent::Discovered { asset } => discovered.push(asset.display_path),
            IndexEvent::Progress(progress) if progress.stage == ScanStage::Completed => {
                final_progress = Some(progress)
            }
            IndexEvent::Completed(_) => break,
            _ => {}
        }
    }
    let summary = scan.join().await.unwrap();

    discovered.sort();
    assert_eq!(discovered, ["a.jpg", "b.jpg", "clip.mp4"]);
    assert_eq!(
        final_progress,
        Some(ScanProgress {
            stage: ScanStage::Completed,
            discovered: 2,
            shaped: 2,
            enriched: 2,
            direct_total: Some(2),
            total: Some(2),
        })
    );
    assert_eq!(summary.discovered, 3);
}

#[tokio::test]
async fn corrupt_shape_uses_four_by_three_and_still_reads_metadata() {
    let fixture = tempfile::tempdir().unwrap();
    std::fs::write(fixture.path().join("broken.jpg"), b"not a jpeg").unwrap();
    let reader = CountingMetadataReader::default();
    let reads = reader.reads.clone();
    let indexer = Indexer::new(reader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    let mut fallback = None;
    while let Some(event) = scan.events.recv().await {
        match event {
            IndexEvent::ShapeFallback { width, height, .. } => fallback = Some((width, height)),
            IndexEvent::Completed(_) => break,
            _ => {}
        }
    }
    scan.join().await.unwrap();
    assert_eq!(fallback, Some((4, 3)));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancellation_stops_a_large_scan_at_a_bounded_partial_result() {
    let fixture = tempfile::tempdir().unwrap();
    for index in 0..1_000 {
        write_png(&fixture.path().join(format!("{index:04}.png")), [1, 2, 3]);
    }
    let indexer = Indexer::new(NoopMetadataReader, empty_policy_engine());
    let mut scan = indexer.start(ScanRequest::new(fixture.path())).unwrap();
    loop {
        match scan.events.recv().await {
            Some(IndexEvent::Discovered { .. }) => break,
            Some(_) => {}
            None => panic!("scan ended before discovering an asset"),
        }
    }

    scan.cancel().unwrap();
    let summary = scan.join().await.unwrap();

    assert!(summary.cancelled);
    assert!(summary.discovered < 1_000);
}

#[tokio::test]
async fn cancellation_completes_when_enrichment_queue_is_backpressured() {
    let fixture = tempfile::tempdir().unwrap();
    for index in 0..160 {
        write_png(&fixture.path().join(format!("{index:04}.png")), [1, 2, 3]);
    }
    let (reader, _notifications, release) = AdmissionReader::new();
    let mut scan = Indexer::new(reader, empty_policy_engine())
        .start(ScanRequest::new(fixture.path()))
        .unwrap();
    let _ = tokio::time::timeout(Duration::from_secs(1), scan.events.recv()).await;
    scan.cancel().unwrap();
    release.release();
    tokio::time::timeout(Duration::from_secs(1), scan.join())
        .await
        .expect("cancelled scan must not deadlock")
        .unwrap();
}

#[tokio::test]
async fn cancellation_join_waits_for_admitted_metadata_without_starting_more_work() {
    let fixture = tempfile::tempdir().unwrap();
    for index in 0..160 {
        write_png(&fixture.path().join(format!("{index:04}.png")), [1, 2, 3]);
    }
    let (reader, notifications, release) = AdmissionReader::new();
    let mut scan = Indexer::new(reader.clone(), empty_policy_engine())
        .start(ScanRequest::new(fixture.path()))
        .unwrap();
    let _notifications = wait_for_starts(notifications, 2).await;

    let mut discovered = 0;
    tokio::time::timeout(Duration::from_secs(1), async {
        while discovered < 70 {
            if let Some(IndexEvent::Discovered { .. }) = scan.events.recv().await {
                discovered += 1;
            }
        }
    })
    .await
    .expect("shape stage should fill the bounded enrichment path");
    assert_eq!(reader.starts.load(Ordering::SeqCst), 2);

    scan.cancel().unwrap();
    let mut join = tokio::spawn(async move { scan.join().await });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut join)
            .await
            .is_err(),
        "join must wait for already-admitted blocking metadata work"
    );
    assert_eq!(
        reader.starts.load(Ordering::SeqCst),
        2,
        "cancellation must not admit more metadata work while quiescing"
    );
    release.release();
    let summary = tokio::time::timeout(Duration::from_secs(1), join)
        .await
        .expect("join did not finish after admitted metadata work quiesced")
        .expect("join task panicked")
        .unwrap();
    assert!(summary.cancelled);
    assert!(summary.discovered < 160);
    assert_eq!(reader.starts.load(Ordering::SeqCst), 2);
}

#[test]
fn catalog_writer_records_discovered_asset_in_its_selection_membership() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Pictures", Path::new("/Pictures")))
        .unwrap();
    let group = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(Path::new("selected")).unwrap(),
            display_path: "selected".to_owned(),
            last_viewed_at: None,
        })
        .unwrap();
    let generation = catalog
        .begin_generation_for_group(library.id, group)
        .unwrap();
    let asset = NewAsset {
        folder_group_id: Some(group),
        ..NewAsset::minimal(
            library.id,
            RelativePathKey::from_relative_path(Path::new("selected/photo.jpg")).unwrap(),
            "selected/photo.jpg",
            MediaKind::Jpeg,
            3,
        )
    };
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&[
            IndexEvent::Discovered {
                asset: asset.clone(),
            },
            IndexEvent::ShapeReady {
                asset_id: asset.id,
                width: 16,
                height: 9,
                orientation: 1,
            },
        ])
        .unwrap();

    assert_eq!(
        catalog
            .wall_page_scoped(
                group,
                GalleryScope::CurrentFolder,
                WallOrder::Provisional,
                None,
                10,
            )
            .unwrap()
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![asset.id]
    );
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().shape_status,
        ShapeStatus::Ready
    );
}

#[tokio::test]
async fn malformed_sidecar_is_warning_event_and_metadata_still_arrives() {
    let fixture = tempfile::tempdir().unwrap();
    let media = fixture.path().join("photo.png");
    write_png(&media, [1, 2, 3]);
    std::fs::write(
        fixture.path().join("photo.xmp"),
        b"<rdf:RDF><rdf:Description>",
    )
    .unwrap();
    let mut scan = Indexer::new(DefaultMetadataReader, empty_policy_engine())
        .start(ScanRequest::new(fixture.path()))
        .unwrap();
    let mut warning = false;
    let mut metadata = false;
    while let Some(event) = scan.events.recv().await {
        match event {
            IndexEvent::Warning {
                code: "malformed_xmp",
                message,
                ..
            } => warning = !message.is_empty(),
            IndexEvent::MetadataReady { .. } => metadata = true,
            IndexEvent::Completed(_) => break,
            _ => {}
        }
    }
    scan.join().await.unwrap();
    assert!(warning);
    assert!(metadata);

    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Pictures", Path::new("/Pictures")))
        .unwrap();
    let generation = catalog.begin_generation(library.id).unwrap();
    let asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("photo.png")).unwrap(),
        "photo.png",
        MediaKind::Png,
        3,
    );
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&[
            IndexEvent::Discovered {
                asset: asset.clone(),
            },
            IndexEvent::Warning {
                asset_id: Some(asset.id),
                code: "malformed_xmp",
                message: "sidecar ended before all elements were closed".into(),
            },
        ])
        .unwrap();
    assert_eq!(catalog.warning_count().unwrap(), 1);
}

#[test]
fn fallback_shape_then_colour_preserves_fallback_state_and_geometry() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Pictures", Path::new("/Pictures")))
        .unwrap();
    let generation = catalog.begin_generation(library.id).unwrap();
    let asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("broken.jpg")).unwrap(),
        "broken.jpg",
        MediaKind::Jpeg,
        3,
    );
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&[
            IndexEvent::Discovered {
                asset: asset.clone(),
            },
            IndexEvent::ShapeFallback {
                asset_id: asset.id,
                width: 4,
                height: 3,
                code: "shape_read_failed",
                message: "bad image".into(),
            },
            IndexEvent::ColourReady {
                asset_id: asset.id,
                representative_rgb: RepresentativeRgb {
                    red: 1,
                    green: 2,
                    blue: 3,
                },
            },
        ])
        .unwrap();
    let stored = catalog.find_asset(asset.id).unwrap().unwrap();
    assert_eq!((stored.width, stored.height), (Some(4), Some(3)));
    assert_eq!(stored.shape_status, photo_catalog::ShapeStatus::Fallback);
}

#[test]
fn catalog_writer_commits_discovery_shape_and_metadata_together() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Pictures", Path::new("/Pictures")))
        .unwrap();
    let relative = RelativePathKey::from_relative_path(Path::new("a.jpg")).unwrap();
    let asset = NewAsset::minimal(library.id, relative, "a.jpg", MediaKind::Jpeg, 10);
    let resolved = ResolvedMetadata {
        rating: Some(4),
        keywords: vec![Keyword {
            normalized: "family".to_owned(),
            display_value: "Family".to_owned(),
            hierarchy: None,
        }],
        provenance: vec![ProvenanceRecord {
            field: "rating".to_owned(),
            source: photo_metadata::MetadataSource::SidecarXmp,
            raw_value: "4".to_owned(),
            chosen: true,
        }],
        ..ResolvedMetadata::default()
    };
    let events = vec![
        IndexEvent::Discovered {
            asset: asset.clone(),
        },
        IndexEvent::ShapeReady {
            asset_id: asset.id,
            width: 100,
            height: 50,
            orientation: 6,
        },
        IndexEvent::ColourReady {
            asset_id: asset.id,
            representative_rgb: RepresentativeRgb {
                red: 1,
                green: 2,
                blue: 3,
            },
        },
        IndexEvent::MetadataReady {
            asset_id: asset.id,
            metadata: resolved,
        },
    ];

    let generation = catalog.begin_generation(library.id).unwrap();
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&events)
        .unwrap();

    let stored = catalog.find_asset(asset.id).unwrap().unwrap();
    assert_eq!((stored.width, stored.height), (Some(100), Some(50)));
    assert_eq!(stored.rating, Some(4));
    assert_eq!(catalog.asset_keywords(asset.id).unwrap(), vec!["Family"]);
}

#[test]
fn catalog_writer_commits_shape_before_later_metadata() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Pictures", Path::new("/Pictures")))
        .unwrap();
    let generation = catalog.begin_generation(library.id).unwrap();
    let relative = RelativePathKey::from_relative_path(Path::new("a.jpg")).unwrap();
    let asset = NewAsset::minimal(library.id, relative, "a.jpg", MediaKind::Jpeg, 10);
    let discovery_and_shape = vec![
        IndexEvent::Discovered {
            asset: asset.clone(),
        },
        IndexEvent::ShapeReady {
            asset_id: asset.id,
            width: 100,
            height: 50,
            orientation: 1,
        },
    ];
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&discovery_and_shape)
        .unwrap();
    let shaped = catalog.find_asset(asset.id).unwrap().unwrap();
    assert_eq!((shaped.width, shaped.height), (Some(100), Some(50)));
    assert_eq!(shaped.rating, None);

    let metadata = ResolvedMetadata {
        rating: Some(4),
        ..ResolvedMetadata::default()
    };
    CatalogWriter::new(&mut catalog, library.id, generation)
        .apply_batch(&[IndexEvent::MetadataReady {
            asset_id: asset.id,
            metadata,
        }])
        .unwrap();
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().rating,
        Some(4)
    );
}

#[test]
fn catalog_writer_rolls_back_an_invalid_record_in_a_batch() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Pictures", Path::new("/Pictures")))
        .unwrap();
    let other_library = catalog
        .add_library(&NewLibrary::configured("Other", Path::new("/Other")))
        .unwrap();
    let generation = catalog.begin_generation(library.id).unwrap();
    let relative = RelativePathKey::from_relative_path(Path::new("a.jpg")).unwrap();
    let valid = NewAsset::minimal(library.id, relative, "a.jpg", MediaKind::Jpeg, 10);
    let invalid_relative = RelativePathKey::from_relative_path(Path::new("bad.jpg")).unwrap();
    let invalid = NewAsset::minimal(
        other_library.id,
        invalid_relative,
        "bad.jpg",
        MediaKind::Jpeg,
        10,
    );

    let result = CatalogWriter::new(&mut catalog, library.id, generation).apply_batch(&[
        IndexEvent::Discovered { asset: valid },
        IndexEvent::Discovered { asset: invalid },
    ]);
    assert!(result.is_err());
    assert_eq!(catalog.asset_count(library.id).unwrap(), 0);
}

#[test]
fn generation_writer_rejects_foreign_shape_colour_metadata_and_warning_targets() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Pictures", Path::new("/Pictures")))
        .unwrap();
    let foreign_library = catalog
        .add_library(&NewLibrary::configured("Other", Path::new("/Other")))
        .unwrap();
    let local = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("local.jpg")).unwrap(),
        "local.jpg",
        MediaKind::Jpeg,
        1,
    );
    let foreign = NewAsset::minimal(
        foreign_library.id,
        RelativePathKey::from_relative_path(Path::new("foreign.jpg")).unwrap(),
        "foreign.jpg",
        MediaKind::Jpeg,
        1,
    );
    catalog.upsert_asset(&local).unwrap();
    catalog.upsert_asset(&foreign).unwrap();
    let generation = catalog.begin_generation(library.id).unwrap();
    let foreign_events = [
        IndexEvent::ShapeReady {
            asset_id: foreign.id,
            width: 1,
            height: 1,
            orientation: 1,
        },
        IndexEvent::ColourReady {
            asset_id: foreign.id,
            representative_rgb: RepresentativeRgb {
                red: 1,
                green: 2,
                blue: 3,
            },
        },
        IndexEvent::MetadataReady {
            asset_id: foreign.id,
            metadata: ResolvedMetadata {
                rating: Some(5),
                ..ResolvedMetadata::default()
            },
        },
        IndexEvent::Warning {
            asset_id: Some(foreign.id),
            code: "foreign",
            message: "foreign".into(),
        },
    ];
    for foreign_event in foreign_events {
        let result = CatalogWriter::new(&mut catalog, library.id, generation).apply_batch(&[
            IndexEvent::MetadataReady {
                asset_id: local.id,
                metadata: ResolvedMetadata {
                    rating: Some(4),
                    ..ResolvedMetadata::default()
                },
            },
            foreign_event,
        ]);
        assert!(result.is_err());
        assert_eq!(catalog.find_asset(local.id).unwrap().unwrap().rating, None);
        assert_eq!(catalog.warning_count().unwrap(), 0);
    }
}

#[test]
fn filename_sidecar_wins_over_stem_sidecar_case_insensitively() {
    let fixture = tempfile::tempdir().unwrap();
    let media = fixture.path().join("Photo.JPG");
    std::fs::write(&media, b"data").unwrap();
    std::fs::write(fixture.path().join("PHOTO.JPG.XMP"), b"filename").unwrap();
    std::fs::write(fixture.path().join("photo.xmp"), b"stem").unwrap();

    let found = photo_indexer::find_sidecar(&media).unwrap();

    assert_eq!(found, Some(fixture.path().join("PHOTO.JPG.XMP")));
}

#[tokio::test]
async fn oriented_image_emits_display_dimensions_and_orientation() {
    let fixture = tempfile::tempdir().unwrap();
    let image_path = fixture.path().join("oriented.jpg");
    std::fs::write(&image_path, oriented_tiff(2, 3, 6)).unwrap();
    let mut scan = Indexer::new(NoopMetadataReader, empty_policy_engine())
        .start(ScanRequest::new(fixture.path()))
        .unwrap();

    let mut observed = None;
    while let Some(event) = scan.events.recv().await {
        if let IndexEvent::ShapeReady {
            width,
            height,
            orientation,
            ..
        } = event
        {
            observed = Some((width, height, orientation));
            break;
        }
    }
    scan.join().await.unwrap();
    assert_eq!(observed, Some((3, 2, 6)));
}

#[cfg(feature = "heic")]
#[tokio::test]
async fn heif_scan_and_derivative_use_the_container_transform_once() {
    let fixture = tempfile::tempdir().unwrap();
    let media = fixture.path().join("portrait-rotated.heic");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../codec/tests/fixtures/heif/portrait-rotated.heic"),
        &media,
    )
    .unwrap();
    let mut scan = Indexer::new(DefaultMetadataReader, empty_policy_engine())
        .start(ScanRequest::new(fixture.path()))
        .unwrap();

    let mut observed = None;
    let mut captured_at = None;
    while let Some(event) = scan.events.recv().await {
        match event {
            IndexEvent::ShapeReady {
                asset_id,
                width,
                height,
                orientation,
            } => observed = Some((asset_id, width, height, orientation)),
            IndexEvent::MetadataReady { metadata, .. } => {
                captured_at = metadata.captured_at.map(|value| value.to_rfc3339());
            }
            _ => {}
        }
    }
    scan.join().await.unwrap();
    let (asset_id, width, height, orientation) = observed.expect("HEIF shape event");
    assert_eq!((width, height, orientation), (100, 28, 1));
    assert_eq!(captured_at.as_deref(), Some("2024-03-04T05:06:07+00:00"));

    let cache = tempfile::tempdir().unwrap();
    let generated = ImageDerivativeGenerator::new(cache.path())
        .unwrap()
        .generate(
            &media,
            &DerivativeSpec {
                asset_id,
                signature: photo_domain::FileSignature {
                    size_bytes: std::fs::metadata(&media).unwrap().len(),
                    modified_unix_ns: 1,
                    sidecar_modified_unix_ns: None,
                },
                media_kind: MediaKind::Heif,
                orientation,
                kind: DerivativeKind::WallThumbnail,
                decoder_version: photo_codec::decoder_fingerprint(MediaKind::Heif)
                    .unwrap()
                    .to_owned(),
                colour_space: "srgb".into(),
                target: DerivativeTarget::LongEdge(1024),
            },
        )
        .unwrap();
    assert_eq!(
        image::image_dimensions(cache.path().join(generated.relative_path)).unwrap(),
        (100, 28)
    );
}

#[tokio::test]
async fn selection_walk_uses_library_relative_identity_and_folder_group() {
    let fixture = tempfile::tempdir().unwrap();
    let library_root = fixture.path().join("library");
    let selection_root = library_root.join("child");
    std::fs::create_dir_all(&selection_root).unwrap();
    let image_path = selection_root.join("a.png");
    write_png(&image_path, [1, 2, 3]);
    let library_id = photo_domain::LibraryId::new();
    let folder_group_id = photo_domain::FolderGroupId::new();
    let mut scan = Indexer::new(NoopMetadataReader, empty_policy_engine())
        .start(
            ScanRequest::new(&selection_root)
                .for_library(library_id)
                .roots(&library_root, &selection_root)
                .for_folder_group(folder_group_id),
        )
        .unwrap();

    let asset = loop {
        match scan.events.recv().await {
            Some(IndexEvent::Discovered { asset }) => break asset,
            Some(_) => {}
            None => panic!("scan ended without discovery"),
        }
    };
    scan.join().await.unwrap();

    let expected_relative = RelativePathKey::from_relative_path(Path::new("child/a.png")).unwrap();
    assert_eq!(asset.relative_path, expected_relative);
    assert_eq!(
        asset.id,
        photo_domain::AssetId::for_path(library_id, &expected_relative)
    );
    assert_eq!(asset.folder_group_id, Some(folder_group_id));
}

#[test]
fn default_reader_merges_embedded_sidecar_and_filesystem_metadata() {
    let fixture = tempfile::tempdir().unwrap();
    let media = fixture.path().join("photo.jpg");
    let sidecar = fixture.path().join("photo.xmp");
    std::fs::write(&media, oriented_tiff(2, 3, 6)).unwrap();
    std::fs::write(
        &sidecar,
        br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="4"/>"#,
    )
    .unwrap();

    let bundle = DefaultMetadataReader.read(&media, Some(&sidecar)).unwrap();
    assert_eq!(bundle.orientation, Some(6));
    assert_eq!(
        bundle
            .ratings
            .iter()
            .map(|value| value.value)
            .collect::<Vec<_>>(),
        vec![4]
    );
    assert!(bundle
        .capture_dates
        .iter()
        .any(|candidate| candidate.source == photo_metadata::MetadataSource::FilesystemModified));
}

#[test]
fn default_reader_warns_on_malformed_xmp_but_keeps_embedded_and_filesystem_values() {
    let fixture = tempfile::tempdir().unwrap();
    let media = fixture.path().join("photo.jpg");
    let sidecar = fixture.path().join("photo.xmp");
    std::fs::write(&media, oriented_tiff(2, 3, 6)).unwrap();
    std::fs::write(&sidecar, b"<rdf:RDF><rdf:Description>").unwrap();

    let bundle = DefaultMetadataReader.read(&media, Some(&sidecar)).unwrap();
    assert_eq!(bundle.orientation, Some(6));
    assert!(
        bundle
            .warnings
            .iter()
            .any(|warning| warning.code == "malformed_xmp")
    );
    assert!(bundle
        .capture_dates
        .iter()
        .any(|candidate| candidate.source == photo_metadata::MetadataSource::FilesystemModified));
}

fn oriented_tiff(width: u16, height: u16, orientation: u16) -> Vec<u8> {
    let mut bytes = vec![b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x03, 0x00];
    let entries = [
        (0x0100_u16, width),
        (0x0101_u16, height),
        (0x0112_u16, orientation),
    ];
    for (tag, value) in entries {
        bytes.extend_from_slice(&tag.to_le_bytes());
        bytes.extend_from_slice(&3_u16.to_le_bytes());
        bytes.extend_from_slice(&1_u32.to_le_bytes());
        bytes.extend_from_slice(&value.to_le_bytes());
        bytes.extend_from_slice(&[0, 0]);
    }
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes
}
