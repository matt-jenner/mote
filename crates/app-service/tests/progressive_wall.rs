use std::collections::BTreeSet;
#[cfg(feature = "heic")]
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;
use std::{io, sync::Once};

use chrono::{FixedOffset, TimeZone};
use image::{ImageBuffer, Rgb};
use photo_app_service::{
    AppConfig, AppService, DerivativeClass, DerivativeReference, DerivativeRequest, MetadataReader,
    OrderState, SortDirection, SourceAvailability, WallAsset, WallMediaKind, WallPage,
    WallQueryRequest, WallShapeState, WallUpdate, WallWarningState,
};
use photo_cache::CacheBudget;
use photo_catalog::{
    Catalog, CatalogWarningRecord, NewDerivative, NewFolderGroup, TerminalDerivativeFailure,
};
use photo_domain::{
    Appearance, AssetId, Availability, DerivativeId, FolderGroupId, GalleryScope, LibraryId,
    MediaKind, RelativePathKey,
};
use photo_metadata::{MetadataBundle, MetadataCandidate, MetadataReadWarning, MetadataSource};
use rusqlite::Connection;
#[cfg(feature = "heic")]
use sha2::{Digest, Sha256};

#[derive(Clone)]
struct BlockingReader {
    gate: Arc<(Mutex<bool>, Condvar)>,
}

#[derive(Clone, Default)]
struct CountingReader(Arc<AtomicUsize>);

impl MetadataReader for CountingReader {
    fn read(
        &self,
        path: &Path,
        sidecar: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        self.0.fetch_add(1, Ordering::SeqCst);
        BlockingReader {
            gate: Arc::new((Mutex::new(true), Condvar::new())),
        }
        .read(path, sidecar)
    }
}

impl CountingReader {
    fn count(&self) -> usize {
        self.0.load(Ordering::SeqCst)
    }
}

impl BlockingReader {
    fn new() -> (Self, Release) {
        let gate = Arc::new((Mutex::new(false), Condvar::new()));
        (Self { gate: gate.clone() }, Release { gate })
    }
}

impl MetadataReader for BlockingReader {
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
        let index = _media_path
            .file_stem()
            .and_then(|stem| {
                stem.to_string_lossy()
                    .split('-')
                    .next_back()?
                    .parse::<i64>()
                    .ok()
            })
            .unwrap_or(0);
        let value = FixedOffset::east_opt(0)
            .unwrap()
            .timestamp_opt(1_700_000_000 + index, 0)
            .single()
            .unwrap();
        Ok(MetadataBundle {
            capture_dates: vec![MetadataCandidate {
                value,
                source: MetadataSource::FilesystemModified,
                raw_value: value.to_rfc3339(),
            }],
            ..MetadataBundle::default()
        })
    }
}

struct Release {
    gate: Arc<(Mutex<bool>, Condvar)>,
}

impl Release {
    fn release(&self) {
        let (lock, changed) = &*self.gate;
        *lock.lock().unwrap() = true;
        changed.notify_all();
    }
}

impl Drop for Release {
    fn drop(&mut self) {
        self.release();
    }
}

struct ProgressiveFixture {
    temp: tempfile::TempDir,
    source: PathBuf,
    config: AppConfig,
}

#[cfg(feature = "heic")]
#[derive(Debug, Eq, PartialEq)]
enum SourcePermissions {
    #[cfg(unix)]
    UnixMode(u32),
    #[cfg(not(unix))]
    ReadOnly(bool),
}

#[cfg(feature = "heic")]
#[derive(Debug, Eq, PartialEq)]
struct SourceFileSnapshot {
    name: OsString,
    sha256: [u8; 32],
    size: u64,
    modified: std::time::SystemTime,
    permissions: SourcePermissions,
}

#[cfg(feature = "heic")]
fn source_file_snapshots(root: &Path) -> Vec<SourceFileSnapshot> {
    let mut paths = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .into_iter()
        .map(|path| {
            let metadata = std::fs::metadata(&path).unwrap();
            let bytes = std::fs::read(&path).unwrap();
            #[cfg(unix)]
            let permissions = {
                use std::os::unix::fs::PermissionsExt;
                SourcePermissions::UnixMode(metadata.permissions().mode())
            };
            #[cfg(not(unix))]
            let permissions = SourcePermissions::ReadOnly(metadata.permissions().readonly());
            SourceFileSnapshot {
                name: path.file_name().unwrap().to_owned(),
                sha256: Sha256::digest(bytes).into(),
                size: metadata.len(),
                // Compare the filesystem's native SystemTime value directly so
                // its own timestamp precision is preserved on every platform.
                modified: metadata.modified().unwrap(),
                permissions,
            }
        })
        .collect()
}

#[cfg(feature = "heic")]
fn heif_fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../codec/tests/fixtures/heif")
        .join(name)
}

fn managed_cache_files(root: &Path) -> BTreeSet<PathBuf> {
    fn visit(directory: &Path, files: &mut BTreeSet<PathBuf>) {
        for entry in std::fs::read_dir(directory).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(&path, files);
            } else {
                files.insert(path);
            }
        }
    }

    let mut files = BTreeSet::new();
    visit(root, &mut files);
    files
}

#[derive(Debug, Eq, PartialEq)]
struct DerivativeSnapshot {
    asset_id: String,
    folder_group_id: String,
    kind: String,
    cache_key: String,
    relative_cache_path: PathBuf,
    size_bytes: u64,
    durable: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct CacheCatalogWarningSnapshot {
    cache_files: BTreeSet<PathBuf>,
    derivatives: Vec<DerivativeSnapshot>,
    source_warnings: Vec<String>,
    asset_warnings: Vec<String>,
}

fn snapshot_cache_catalog_warnings(
    config: &AppConfig,
    asset_id: &str,
) -> CacheCatalogWarningSnapshot {
    let catalog = Catalog::open(&config.catalog_path()).unwrap();
    let active_selection = catalog.load_app_state().unwrap().active_selection.unwrap();
    let asset_id = AssetId::from_uuid(uuid::Uuid::parse_str(asset_id).unwrap());
    let active_group = catalog
        .folder_group_for_path(
            active_selection.library_id,
            &active_selection.relative_folder,
        )
        .unwrap()
        .unwrap();
    let source_warnings = catalog
        .source_warning_summaries(active_selection.library_id)
        .unwrap()
        .into_iter()
        .map(|warning| warning.code)
        .collect();
    let asset_warnings = catalog
        .wall_records_for_assets(active_group, &[asset_id])
        .unwrap()
        .into_iter()
        .filter_map(|record| record.warning_code)
        .collect();
    let derivatives = catalog
        .all_derivatives()
        .unwrap()
        .into_iter()
        .map(|record| DerivativeSnapshot {
            asset_id: record.asset_id.as_uuid().hyphenated().to_string(),
            folder_group_id: record.folder_group_id.as_uuid().hyphenated().to_string(),
            kind: record.kind,
            cache_key: record.cache_key,
            relative_cache_path: record.relative_cache_path,
            size_bytes: record.size_bytes,
            durable: record.durable,
        })
        .collect();
    CacheCatalogWarningSnapshot {
        cache_files: managed_cache_files(config.cache_dir()),
        derivatives,
        source_warnings,
        asset_warnings,
    }
}

fn seed_evictable_screen_preview_and_warning(fixture: &ProgressiveFixture, asset_id: &str) {
    let asset_id = AssetId::from_uuid(uuid::Uuid::parse_str(asset_id).unwrap());
    let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let active_selection = catalog.load_app_state().unwrap().active_selection.unwrap();
    let active_group = catalog
        .folder_group_for_path(
            active_selection.library_id,
            &active_selection.relative_folder,
        )
        .unwrap()
        .unwrap();
    let evictable_group = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: active_selection.library_id,
            relative_path: RelativePathKey::from_relative_path(Path::new("evictable")).unwrap(),
            display_path: "evictable".to_owned(),
            last_viewed_at: Some(1),
        })
        .unwrap();
    assert_ne!(evictable_group, active_group);
    let relative_cache_path = PathBuf::from("evictable/old-preview.jpg");
    std::fs::create_dir_all(
        fixture
            .config
            .cache_dir()
            .join(relative_cache_path.parent().unwrap()),
    )
    .unwrap();
    std::fs::write(
        fixture.config.cache_dir().join(&relative_cache_path),
        vec![0_u8; 1_000_000],
    )
    .unwrap();
    catalog
        .insert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id,
            folder_group_id: evictable_group,
            kind: "screen_preview".to_owned(),
            cache_key: "evictable-screen-preview".to_owned(),
            relative_cache_path,
            size_bytes: 1_000_000,
            durable: false,
            created_at: 1,
        })
        .unwrap();
    catalog
        .record_warning_once(&CatalogWarningRecord {
            library_id: active_selection.library_id,
            asset_id: None,
            code: "screen_preview_cache_unavailable".to_owned(),
            message: "seeded test warning".to_owned(),
        })
        .unwrap();
    catalog
        .record_warning_once(&CatalogWarningRecord {
            library_id: active_selection.library_id,
            asset_id: Some(asset_id),
            code: "derivative_generation_failed".to_owned(),
            message: "seeded asset warning".to_owned(),
        })
        .unwrap();
}

#[derive(Debug, Eq, PartialEq)]
struct WarningClearObservation {
    selection_id: String,
    source_id: String,
    asset_id: Option<String>,
    code: String,
}

#[derive(Debug, Eq, PartialEq)]
struct DerivativeUpdateObservation {
    screen_publications: Vec<(String, DerivativeReference)>,
    warning_count: usize,
    warning_clears: Vec<WarningClearObservation>,
}

fn derivative_update_observation(
    updates: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
) -> DerivativeUpdateObservation {
    let mut screen_publications = Vec::new();
    let mut warning_count = 0;
    let mut warning_clears = Vec::new();
    while let Ok(event) = updates.try_recv() {
        match event {
            WallUpdate::DerivativesReady {
                selection_id,
                derivatives,
                ..
            } => {
                screen_publications.extend(
                    derivatives
                        .into_iter()
                        .filter(|derivative| derivative.kind == DerivativeClass::ScreenPreview)
                        .map(|derivative| (selection_id.clone(), derivative)),
                );
            }
            WallUpdate::Warning { .. } => warning_count += 1,
            WallUpdate::WarningCleared {
                selection_id,
                source_id,
                asset_id,
                code,
            } => warning_clears.push(WarningClearObservation {
                selection_id,
                source_id,
                asset_id,
                code,
            }),
            _ => {}
        }
    }
    DerivativeUpdateObservation {
        screen_publications,
        warning_count,
        warning_clears,
    }
}

fn derivative_update_counts(
    updates: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
) -> (usize, usize, usize) {
    let observation = derivative_update_observation(updates);
    (
        observation.screen_publications.len(),
        observation.warning_count,
        observation.warning_clears.len(),
    )
}

impl ProgressiveFixture {
    fn new(count: usize) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        for index in 0..count {
            let image = ImageBuffer::from_pixel(16, 12, Rgb([index as u8, 10, 20]));
            image
                .save(source.join(format!("photo-{index:03}.jpg")))
                .unwrap();
        }
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        Self {
            temp,
            source,
            config,
        }
    }

    fn video_only() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("videos");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("clip.mp4"), b"not a video").unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        Self {
            temp,
            source,
            config,
        }
    }

    fn capability_matrix() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        ImageBuffer::from_pixel(16, 12, Rgb([20_u8, 40, 60]))
            .save(source.join("displayable.jpg"))
            .unwrap();
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../codec/tests/fixtures/heif/iphone-8bit.heic"),
            source.join("phone.HEIC"),
        )
        .unwrap();
        std::fs::write(source.join("future.avif"), b"avif fixture").unwrap();
        std::fs::write(source.join("camera.dng"), b"raw fixture").unwrap();
        std::fs::write(source.join("clip.mp4"), b"video fixture").unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        Self {
            temp,
            source,
            config,
        }
    }

    fn mixed_collection() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        std::fs::create_dir_all(&source).unwrap();
        ImageBuffer::from_pixel(16, 12, Rgb([10_u8, 20, 30]))
            .save(source.join("a.jpg"))
            .unwrap();
        std::fs::write(source.join("corrupt.jpg"), b"not a jpeg").unwrap();
        ImageBuffer::from_pixel(16, 12, Rgb([50_u8, 50, 50]))
            .save(source.join("offline.jpg"))
            .unwrap();
        ImageBuffer::from_pixel(16, 12, Rgb([30_u8, 20, 10]))
            .save(source.join("b.jpg"))
            .unwrap();
        std::fs::write(source.join("clip.mp4"), b"not a video").unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        Self {
            temp,
            source,
            config,
        }
    }

    fn service(&self, reader: BlockingReader) -> AppService {
        AppService::open_with_reader(self.config.clone(), Arc::new(reader)).unwrap()
    }
}

async fn recv_until<F>(
    receiver: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
    mut predicate: F,
) -> WallUpdate
where
    F: FnMut(&WallUpdate) -> bool,
{
    const UPDATE_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
    let mut last = None;
    loop {
        match tokio::time::timeout(UPDATE_WAIT_TIMEOUT, receiver.recv()).await {
            Ok(Ok(event)) => {
                if predicate(&event) {
                    return event;
                }
                last = Some(event);
            }
            Ok(Err(error)) => panic!("update receiver failed after {last:?}: {error}"),
            Err(_) => panic!("timed out waiting for update, last event: {last:?}"),
        }
    }
}

async fn recv_derivatives_until(
    receiver: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
    kind: DerivativeClass,
    expected: usize,
) -> Vec<DerivativeReference> {
    let mut references = Vec::new();
    loop {
        let event = recv_until(receiver, |event| {
            matches!(event, WallUpdate::DerivativesReady { derivatives, .. }
                if derivatives.iter().any(|item| item.kind == kind))
        })
        .await;
        if let WallUpdate::DerivativesReady { derivatives, .. } = event {
            references.extend(derivatives.into_iter().filter(|item| item.kind == kind));
        }
        if references.len() >= expected {
            return references;
        }
    }
}

fn wall_cache_key(asset: &photo_catalog::AssetRecord) -> String {
    photo_cache::DerivativeKey::compute(&photo_cache::DerivativeSpec {
        asset_id: asset.id,
        signature: asset.signature,
        media_kind: asset.media_kind,
        orientation: asset.orientation.unwrap_or(1),
        kind: photo_cache::DerivativeKind::WallThumbnail,
        decoder_version: photo_codec::decoder_fingerprint(asset.media_kind)
            .unwrap_or_default()
            .to_owned(),
        colour_space: "srgb".to_owned(),
        target: photo_cache::DerivativeTarget::LongEdge(1024),
    })
    .as_str()
    .to_owned()
}

fn query(direction: SortDirection) -> WallQueryRequest {
    WallQueryRequest {
        cursor: None,
        limit: 50,
        direction,
    }
}

#[cfg(feature = "heic")]
async fn wait_for_scan_cleanup(service: &AppService) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while service.active_scan_count_test() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("settled scan did not release its active scan state");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn settled_wall_total_includes_terminal_thumbnail_failures() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;

    let initial = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    let asset_id = AssetId::from_uuid(uuid::Uuid::parse_str(&initial.items[0].id).unwrap());
    Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .record_terminal_derivative_failure(&TerminalDerivativeFailure {
            asset_id,
            kind: "wall_thumbnail".to_owned(),
            cache_key: "terminal-test-key".to_owned(),
            availability: Availability::Available,
            failure_code: "derivative_generation_terminal".to_owned(),
            occurred_at: 1,
        })
        .unwrap();

    let filtered = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(filtered.items.len(), 1);
    assert_eq!(filtered.total_count, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reopening_clears_failures_from_an_older_decoder_key() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let page = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    let asset_id = AssetId::from_uuid(uuid::Uuid::parse_str(&page.items[0].id).unwrap());
    let selection = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .load_app_state()
        .unwrap()
        .active_selection
        .unwrap();
    drop(service);

    let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    catalog
        .record_terminal_derivative_failure(&TerminalDerivativeFailure {
            asset_id,
            kind: "wall_thumbnail".to_owned(),
            cache_key: "legacy-decoder-key".to_owned(),
            availability: Availability::Available,
            failure_code: "derivative_generation_terminal".to_owned(),
            occurred_at: 1,
        })
        .unwrap();
    catalog
        .record_warning_once(&CatalogWarningRecord {
            library_id: selection.library_id,
            asset_id: Some(asset_id),
            code: "derivative_generation_terminal".to_owned(),
            message: "legacy decoder failure".to_owned(),
        })
        .unwrap();
    drop(catalog);

    let reopened = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let recovered = reopened
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();

    assert_eq!(recovered.total_count, 1);
    assert_eq!(recovered.preview_counts.wall_failed, 0);
    assert!(recovered.items[0].warning.is_none());
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    assert_eq!(catalog.warning_count().unwrap(), 0);
    assert!(
        catalog
            .find_terminal_derivative_failure(
                asset_id,
                "wall_thumbnail",
                "legacy-decoder-key",
                Availability::Available,
            )
            .unwrap()
            .is_none()
    );
}

#[test]
fn same_library_parent_and_child_have_distinct_opaque_selection_identity() {
    let fixture = ProgressiveFixture::new(1);
    let child = fixture.source.join("child");
    std::fs::create_dir_all(&child).unwrap();
    let service = AppService::open(fixture.config.clone()).unwrap();

    let parent = service.open_recent(&fixture.source).unwrap();
    let child_state = service.open_recent(&child).unwrap();
    let parent_source = parent.active_source.unwrap();
    let child_source = child_state.active_source.unwrap();

    assert_eq!(parent_source.id, child_source.id);
    assert_ne!(parent_source.selection_id, child_source.selection_id);
    assert!(!child_source.selection_id.contains("child"));
    assert!(!child_source.selection_id.contains('/'));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn gallery_scope_filters_cached_pages_and_progressive_batches_without_rescanning() {
    let fixture = ProgressiveFixture::new(1);
    let child = fixture.source.join("child");
    std::fs::create_dir(&child).unwrap();
    ImageBuffer::from_pixel(16, 12, Rgb([80_u8, 40, 20]))
        .save(child.join("nested.jpg"))
        .unwrap();
    let reader = CountingReader::default();
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(reader.clone())).unwrap();
    service
        .update_gallery_scope(GalleryScope::CurrentFolder)
        .await
        .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();

    let mut published = Vec::new();
    loop {
        match recv_until(&mut updates, |_| true).await {
            WallUpdate::CatalogBatch { assets, .. } => {
                published.extend(assets.into_iter().map(|asset| asset.display_name));
            }
            WallUpdate::MetadataSettled { .. } => break,
            _ => {}
        }
    }
    assert_eq!(published, ["photo-000.jpg"]);
    let current = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(
        current
            .items
            .iter()
            .map(|asset| asset.display_name.as_str())
            .collect::<Vec<_>>(),
        ["photo-000.jpg"]
    );
    let reads_after_scan = reader.count();

    service
        .update_gallery_scope(GalleryScope::IncludeSubfolders)
        .await
        .unwrap();
    let recursive = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    let mut recursive_names = recursive
        .items
        .iter()
        .map(|asset| asset.display_name.as_str())
        .collect::<Vec<_>>();
    recursive_names.sort_unstable();
    assert_eq!(recursive_names, ["nested.jpg", "photo-000.jpg"]);
    assert_eq!(reader.count(), reads_after_scan);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn uncached_folder_emits_geometry_before_metadata_settles_and_reopens_from_cache() {
    let fixture = ProgressiveFixture::new(12);
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();

    let provisional = recv_until(
        &mut updates,
        |event| matches!(event, WallUpdate::CatalogBatch { assets, .. } if !assets.is_empty()),
    )
    .await;
    match provisional {
        WallUpdate::CatalogBatch {
            assets,
            order_state,
            ..
        } => {
            assert!(assets.len() >= 4);
            assert_eq!(order_state, OrderState::Provisional);
            assert!(
                assets
                    .iter()
                    .all(|asset| asset.width > 0 && asset.height > 0)
            );
        }
        _ => unreachable!(),
    }
    let page = tokio::time::timeout(
        Duration::from_secs(2),
        service.query_wall(query(SortDirection::NewestFirst)),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(page.items.len() >= 4);
    assert_eq!(page.order_state, OrderState::Provisional);
    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    assert_eq!(
        service
            .query_wall(query(SortDirection::NewestFirst))
            .await
            .unwrap()
            .order_state,
        OrderState::Settled
    );
    let settled_page = service
        .query_wall(query(SortDirection::NewestFirst))
        .await
        .unwrap();
    assert!(settled_page.items.iter().all(|asset| {
        asset.display_name.ends_with(".jpg")
            && asset.media_kind == WallMediaKind::Jpeg
            && asset.provisional_order > 0
            && asset.date_state == OrderState::Settled
            && asset.representative_rgb.is_some()
            && asset.shape_state == WallShapeState::Ready
            && asset.availability == SourceAvailability::Available
    }));
    drop(service);

    let reopened =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(BlockingReader::new().0))
            .unwrap();
    let page = reopened
        .query_wall(query(SortDirection::NewestFirst))
        .await
        .unwrap();
    assert_eq!(page.items.len(), 12);
    assert_eq!(page.order_state, OrderState::Settled);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn progressive_flush_arrives_within_the_bounded_window() {
    let fixture = ProgressiveFixture::new(12);
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();

    let first_batch = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            let event = updates.recv().await.unwrap();
            if matches!(&event, WallUpdate::CatalogBatch { assets, .. } if !assets.is_empty()) {
                break event;
            }
        }
    })
    .await;
    release.release();
    let first_batch =
        first_batch.expect("progressive catalog batch exceeded its bounded flush window");
    assert!(matches!(first_batch, WallUpdate::CatalogBatch { .. }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn open_recent_starts_the_selected_folder_scan() {
    let fixture = ProgressiveFixture::new(4);
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = service.subscribe_wall_updates();

    service.open_recent(&fixture.source).unwrap();

    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    assert_eq!(
        service
            .query_wall(query(SortDirection::OldestFirst))
            .await
            .unwrap()
            .items
            .len(),
        4
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn completed_sibling_group_does_not_settle_a_new_child_selection() {
    let fixture = ProgressiveFixture::new(2);
    let child = fixture.source.join("child");
    std::fs::create_dir_all(&child).unwrap();
    for index in 0..2 {
        ImageBuffer::from_pixel(14, 9, Rgb([12_u8, index as u8, 30]))
            .save(child.join(format!("child-{index}.jpg")))
            .unwrap();
    }
    let first =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = first.subscribe_wall_updates();
    first.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    first.shutdown().await;
    first.wait_for_collection_drivers_quiescent_test().await;
    first.wait_for_derivative_tasks_quiescent_test().await;
    drop(first);

    let (reader, release) = BlockingReader::new();
    let service = AppService::open_with_reader(fixture.config.clone(), Arc::new(reader)).unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&child).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::CatalogBatch { .. })
    })
    .await;

    assert_eq!(
        service
            .query_wall(query(SortDirection::OldestFirst))
            .await
            .unwrap()
            .order_state,
        OrderState::Provisional
    );
    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    assert_eq!(
        service
            .query_wall(query(SortDirection::OldestFirst))
            .await
            .unwrap()
            .order_state,
        OrderState::Settled
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_cancelled_scan_cannot_settle_the_replacement_source() {
    let fixture = ProgressiveFixture::new(12);
    let source_b = fixture.temp.path().join("replacement");
    std::fs::create_dir_all(&source_b).unwrap();
    for index in 0..4 {
        let image = ImageBuffer::from_pixel(20, 10, Rgb([30, index as u8, 10]));
        image
            .save(source_b.join(format!("replacement-{index}.jpg")))
            .unwrap();
    }
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::CatalogBatch { .. })
    })
    .await;
    service.start_scan(&source_b).await.unwrap();
    release.release();
    let mut settlements = 0;
    let mut batches = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(2), updates.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WallUpdate::MetadataSettled { .. } => {
                settlements += 1;
                if settlements == 1 {
                    break;
                }
            }
            WallUpdate::CatalogBatch { assets, .. } => batches.extend(assets),
            _ => {}
        }
    }
    assert_eq!(settlements, 1);
    assert!(!batches.is_empty());
    let page = service
        .query_wall(query(SortDirection::NewestFirst))
        .await
        .unwrap();
    assert_eq!(page.items.len(), 4);
    let active_ids = page
        .items
        .iter()
        .map(|asset| asset.id.as_str())
        .collect::<std::collections::HashSet<_>>();
    assert!(
        batches
            .iter()
            .all(|asset| active_ids.contains(asset.id.as_str())),
        "a replaced scan published a catalog batch for the old selection"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn changing_direction_queries_sqlite_without_starting_another_scan() {
    let fixture = ProgressiveFixture::new(12);
    let reader = CountingReader::default();
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(reader.clone())).unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let scans_before = reader.count();
    let newest = service
        .query_wall(query(SortDirection::NewestFirst))
        .await
        .unwrap();
    assert!(
        newest
            .items
            .windows(2)
            .all(|pair| pair[0].captured_at_utc >= pair[1].captured_at_utc)
    );
    assert_eq!(reader.count(), scans_before);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn wall_cursor_is_rejected_after_switching_folder_groups() {
    let fixture = ProgressiveFixture::new(4);
    let other = fixture.temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    for index in 0..2 {
        ImageBuffer::from_pixel(12, 8, Rgb([40_u8, index as u8, 5]))
            .save(other.join(format!("other-{index}.jpg")))
            .unwrap();
    }
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let first = service
        .query_wall(WallQueryRequest {
            cursor: None,
            limit: 1,
            direction: SortDirection::OldestFirst,
        })
        .await
        .unwrap();
    let stale_cursor = first.next_cursor.unwrap();

    service.start_scan(&other).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let error = service
        .query_wall(WallQueryRequest {
            cursor: Some(stale_cursor),
            limit: 1,
            direction: SortDirection::OldestFirst,
        })
        .await
        .unwrap_err();

    assert!(matches!(
        error,
        photo_app_service::AppServiceError::InvalidCursor
    ));
}

#[tokio::test]
async fn desktop_bootstrap_unavailable_reopen_preserves_the_view_and_cached_catalog() {
    let fixture = ProgressiveFixture::new(12);
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::CatalogBatch { .. })
    })
    .await;
    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    drop(service);

    let unavailable = fixture.temp.path().join("photos-offline");
    std::fs::rename(&fixture.source, &unavailable).unwrap();
    let reopened =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let bootstrap = reopened.desktop_bootstrap().await.unwrap();
    assert!(bootstrap.active_source.is_some());
    assert_eq!(bootstrap.saved_folders.entries.len(), 1);
    assert!(bootstrap.saved_folders.active_entry_id.is_some());
    assert_eq!(
        reopened
            .query_wall(query(SortDirection::NewestFirst))
            .await
            .unwrap()
            .items
            .len(),
        12
    );
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let library = catalog.list_libraries().unwrap().remove(0);
    assert_eq!(
        catalog.assets(library.id).unwrap().len(),
        12,
        "clearing the view must preserve the cached catalog"
    );
    std::fs::rename(unavailable, &fixture.source).unwrap();
}

#[tokio::test]
async fn desktop_bootstrap_selects_naturally_first_saved_folder() {
    let fixture = ProgressiveFixture::new(0);
    let service = AppService::open(fixture.config.clone()).unwrap();
    let mut first_id = String::new();
    for name in ["Album 10", "album 2", "Zoo"] {
        let path = fixture.temp.path().join(name);
        std::fs::create_dir(&path).unwrap();
        let bootstrap = service.open_recent(&path).unwrap();
        if name == "album 2" {
            first_id = bootstrap.saved_folders.active_entry_id.unwrap();
        }
    }
    let bootstrap = service.desktop_bootstrap().await.unwrap();
    assert_eq!(
        bootstrap.saved_folders.active_entry_id.as_deref(),
        Some(first_id.as_str())
    );
    assert_eq!(bootstrap.active_source.unwrap().display_name, "album 2");
}

#[tokio::test]
async fn root_first_unavailable_selected_child_marks_only_its_group_offline() {
    let fixture = ProgressiveFixture::new(4);
    let child = fixture.source.join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::copy(
        fixture.source.join("photo-000.jpg"),
        child.join("child.jpg"),
    )
    .unwrap();
    let reader = CountingReader::default();
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(reader.clone())).unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    service.start_scan(&child).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let before = service.bootstrap().unwrap();
    let reads_before_check = reader.count();
    let unavailable = fixture.source.join("child-offline");
    std::fs::rename(&child, &unavailable).unwrap();
    let ids: Vec<_> = before
        .saved_folders
        .entries
        .iter()
        .map(|entry| entry.id.clone())
        .collect();
    let checked = service.check_saved_folders(&ids).await.unwrap();
    assert_eq!(
        checked.active_entry_id,
        before.saved_folders.active_entry_id
    );
    for entry in &checked.entries {
        let expected = if entry.name == "child" {
            photo_app_service::FolderAccessState::Missing
        } else {
            photo_app_service::FolderAccessState::Available
        };
        assert_eq!(checked.access[&entry.folder_id].state, expected);
    }
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let active = catalog.load_app_state().unwrap().active_selection.unwrap();
    let assets = catalog.assets(active.library_id).unwrap();
    assert_eq!(assets.len(), 5);
    for asset in &assets {
        let expected = if asset.display_path == "child/child.jpg" {
            Availability::RootOffline
        } else {
            Availability::Available
        };
        assert_eq!(asset.availability, expected, "{}", asset.display_path);
    }
    assert_eq!(
        reader.count(),
        reads_before_check,
        "folder access checks must not probe individual assets"
    );
    let active_group = catalog
        .folder_group_for_path(active.library_id, &active.relative_folder)
        .unwrap()
        .unwrap();
    let recovery = catalog
        .folder_group_recovery_state(active.library_id, active_group)
        .unwrap();
    assert!(
        recovery.requested > recovery.reconciled,
        "an unavailable saved-folder check must persist its pending recovery"
    );
    assert_eq!(
        service
            .query_wall(query(SortDirection::NewestFirst))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    std::fs::rename(unavailable, child).unwrap();
}

#[tokio::test]
async fn root_first_offline_library_marks_sibling_assets_offline() {
    let fixture = ProgressiveFixture::new(2);
    let reader = CountingReader::default();
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(reader.clone())).unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |e| {
        matches!(e, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let mut child_entry = String::new();
    for name in ["child-a", "child-b"] {
        let child = fixture.source.join(name);
        std::fs::create_dir(&child).unwrap();
        std::fs::copy(
            fixture.source.join("photo-000.jpg"),
            child.join("photo.jpg"),
        )
        .unwrap();
        let bootstrap = service.start_scan(&child).await.unwrap();
        recv_until(&mut updates, |e| {
            matches!(e, WallUpdate::MetadataSettled { .. })
        })
        .await;
        child_entry = bootstrap.saved_folders.active_entry_id.unwrap();
    }
    let before = reader.count();
    let offline = fixture.temp.path().join("offline");
    std::fs::rename(&fixture.source, &offline).unwrap();
    service.check_saved_folders(&[child_entry]).await.unwrap();
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let library = catalog.list_libraries().unwrap().remove(0);
    let assets = catalog.assets(library.id).unwrap();
    assert_eq!(assets.len(), 4);
    assert!(
        assets
            .iter()
            .all(|a| a.availability == Availability::RootOffline),
        "root failure must cover sibling groups"
    );
    assert_eq!(
        reader.count(),
        before,
        "failed prerequisites must not admit asset readers"
    );
    let event = tokio::time::timeout(Duration::from_millis(100), async {
        loop {
            let event = updates.recv().await.unwrap();
            if matches!(event, WallUpdate::SourceUnavailable { .. }) {
                break event;
            }
        }
    })
    .await
    .expect("confirmed root failure must notify the selected view");
    let WallUpdate::SourceUnavailable { selection_id, .. } = event else {
        unreachable!()
    };
    assert_eq!(
        selection_id,
        service
            .bootstrap()
            .unwrap()
            .active_source
            .unwrap()
            .selection_id
    );
}

#[cfg(unix)]
#[tokio::test]
async fn root_first_unreadable_asset_publishes_warning_and_keeps_sibling_available() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = ProgressiveFixture::new(2);
    std::fs::set_permissions(
        fixture.source.join("photo-000.jpg"),
        std::fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    let service = AppService::open(fixture.config.clone()).unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    let mut warnings = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), updates.recv())
            .await
            .unwrap()
            .unwrap();
        if let WallUpdate::Warning {
            asset_id, warning, ..
        } = &event
        {
            warnings.push((asset_id.clone(), warning.code.clone()));
        }
        if matches!(event, WallUpdate::MetadataSettled { .. }) {
            break;
        }
    }
    assert!(
        warnings
            .iter()
            .any(|(id, code)| id.is_some() && code == "source_unreadable")
    );
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let library = catalog.list_libraries().unwrap().remove(0);
    let assets = catalog.assets(library.id).unwrap();
    assert_eq!(
        assets
            .iter()
            .filter(|a| a.availability == Availability::Unreadable)
            .count(),
        1
    );
    assert_eq!(
        assets
            .iter()
            .filter(|a| a.availability == Availability::Available)
            .count(),
        1
    );
}

#[tokio::test]
async fn root_first_missing_selected_root_marks_library_offline() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open(fixture.config.clone()).unwrap();
    let mut updates = service.subscribe_wall_updates();
    let saved = service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    std::fs::rename(&fixture.source, fixture.temp.path().join("offline")).unwrap();
    service
        .check_saved_folders(&[saved.saved_folders.active_entry_id.unwrap()])
        .await
        .unwrap();
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    assert_eq!(
        catalog.list_libraries().unwrap()[0].availability,
        Availability::RootOffline
    );
}

#[tokio::test]
async fn root_first_readable_folders_return_cached_rows_while_asset_metadata_is_blocked() {
    let fixture = ProgressiveFixture::new(2);
    let service = AppService::open(fixture.config.clone()).unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    service.wait_for_collection_drivers_quiescent_test().await;
    service.wait_for_derivative_tasks_quiescent_test().await;
    drop(service);
    let (reader, release) = BlockingReader::new();
    let reopened = fixture.service(reader);
    let mut updates = reopened.subscribe_wall_updates();
    reopened.desktop_bootstrap().await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::CatalogBatch { .. })
    })
    .await;
    let page = tokio::time::timeout(
        Duration::from_millis(100),
        reopened.query_wall(query(SortDirection::OldestFirst)),
    )
    .await
    .expect("cached wall query must not wait for asset metadata")
    .unwrap();
    assert_eq!(page.items.len(), 2);
    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_folders_keeps_previous_indexing_alive() {
    let fixture = ProgressiveFixture::new(8);
    let other = fixture.temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    ImageBuffer::from_pixel(12, 8, Rgb([10_u8, 20, 30]))
        .save(other.join("other.jpg"))
        .unwrap();
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    let first = service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::CatalogBatch { .. })
    })
    .await;
    let first_entry = first
        .saved_folders
        .entries
        .iter()
        .find(|entry| Some(&entry.id) == first.saved_folders.active_entry_id.as_ref())
        .unwrap();
    let group = FolderGroupId::from_uuid(uuid::Uuid::parse_str(&first_entry.folder_id).unwrap());
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let library = catalog.folder_group(group).unwrap().unwrap().library_id;
    service.start_scan(&other).await.unwrap();
    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !catalog
            .has_completed_generation_for_group(library, group)
            .unwrap()
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("folder A must finish after switching to B");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if catalog
                .all_derivatives()
                .unwrap()
                .iter()
                .filter(|record| record.folder_group_id == group && record.kind == "screen_preview")
                .count()
                == 8
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("A must finish its full collection derivatives without revisiting");
    let returned = service
        .activate_saved_folder(&first_entry.id)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        returned.active_source.unwrap().selection_id,
        first.active_source.unwrap().selection_id
    );
    assert_eq!(
        service
            .query_wall(query(SortDirection::OldestFirst))
            .await
            .unwrap()
            .items
            .len(),
        8
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_folders_return_to_running_folder_reuses_generation_and_rebinds_events() {
    let fixture = ProgressiveFixture::new(8);
    let other = fixture.temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    let first = service.start_scan(&fixture.source).await.unwrap();
    let event = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::CatalogBatch { .. })
    })
    .await;
    let WallUpdate::CatalogBatch {
        generation: original_generation,
        ..
    } = event
    else {
        unreachable!()
    };
    service.start_scan(&other).await.unwrap();
    let returned = service
        .activate_saved_folder(first.saved_folders.active_entry_id.as_ref().unwrap())
        .await
        .unwrap()
        .unwrap();
    let current_id = returned.active_source.unwrap().selection_id;
    while updates.try_recv().is_ok() {}
    release.release();
    let event = recv_until(&mut updates, |event| matches!(event, WallUpdate::MetadataSettled { selection_id, .. } if selection_id == &current_id)).await;
    let WallUpdate::MetadataSettled { generation, .. } = event else {
        unreachable!()
    };
    assert_eq!(
        generation, original_generation,
        "returning to a running folder must not start a second scan"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scan_commits_at_most_two_hundred_events_per_transaction() {
    let fixture = ProgressiveFixture::new(220);
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    let mut largest = 0;
    loop {
        match tokio::time::timeout(Duration::from_secs(2), updates.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WallUpdate::CatalogBatch { assets, .. } => largest = largest.max(assets.len()),
            WallUpdate::MetadataSettled { .. } => break,
            _ => {}
        }
    }
    assert!(largest <= 200, "catalog batch contained {largest} assets");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scan_publishes_geometry_batches_beyond_the_first_wall_page() {
    let fixture = ProgressiveFixture::new(260);
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    let mut published = std::collections::HashSet::new();
    loop {
        match tokio::time::timeout(Duration::from_secs(2), updates.recv())
            .await
            .unwrap()
            .unwrap()
        {
            WallUpdate::CatalogBatch { assets, .. } => {
                published.extend(assets.into_iter().map(|asset| asset.id));
            }
            WallUpdate::MetadataSettled { .. } => break,
            _ => {}
        }
    }

    assert_eq!(published.len(), 260);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_scan_updates_report_the_persisted_catalogue_count() {
    let fixture = ProgressiveFixture::new(1);
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    let event = recv_until(
        &mut updates,
        |event| matches!(event, WallUpdate::CatalogBatch { assets, .. } if !assets.is_empty()),
    )
    .await;
    let WallUpdate::CatalogBatch { progress, .. } = event else {
        unreachable!()
    };
    let page = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();

    assert_eq!(progress.indexed_count, Some(page.indexed_count));
    assert_eq!(progress.direct_indexed_count, Some(page.indexed_count));
    release.release();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn metadata_settlement_emits_once_even_with_duplicate_completion_observation() {
    let fixture = ProgressiveFixture::new(12);
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::CatalogBatch { .. })
    })
    .await;
    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut settlements = 0;
    while let Ok(event) = updates.try_recv() {
        if matches!(event, WallUpdate::MetadataSettled { .. }) {
            settlements += 1;
        }
    }
    assert_eq!(settlements, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_visible_request_produces_one_ready_batch() {
    let fixture = ProgressiveFixture::new(8);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids.clone()))
        .await
        .unwrap();
    let ready = recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 8).await;
    assert_eq!(ready.len(), 8);
    assert_eq!(
        ready
            .iter()
            .map(|d| d.asset_id.clone())
            .collect::<std::collections::HashSet<_>>(),
        ids.iter().cloned().collect()
    );
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids))
        .await
        .unwrap();
    let second = recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 8).await;
    assert_eq!(second.len(), 8);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_visible_screen_preview_request() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();

    service
        .request_derivatives(DerivativeRequest {
            asset_ids: vec![asset_id.clone()],
            priority: photo_app_service::DerivativePriority::Visible,
            kind: DerivativeClass::ScreenPreview,
        })
        .await
        .unwrap();

    let ready = recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 1).await;
    assert_eq!(ready[0].asset_id, asset_id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn screen_preview_request_publishes_wall_thumbnail_before_screen_preview() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();

    service
        .request_derivatives(DerivativeRequest {
            asset_ids: vec![asset_id.clone()],
            priority: photo_app_service::DerivativePriority::Visible,
            kind: DerivativeClass::ScreenPreview,
        })
        .await
        .unwrap();
    let mut first_kind = None;
    let mut screen_seen = false;
    loop {
        let event = recv_until(&mut updates, |event| {
            matches!(event, WallUpdate::DerivativesReady { .. })
        })
        .await;
        let WallUpdate::DerivativesReady { derivatives, .. } = event else {
            unreachable!();
        };
        for derivative in derivatives {
            if derivative.asset_id != asset_id {
                continue;
            }
            first_kind.get_or_insert(derivative.kind);
            if derivative.kind == DerivativeClass::ScreenPreview {
                screen_seen = true;
            }
        }
        if screen_seen {
            break;
        }
    }

    assert_eq!(first_kind, Some(DerivativeClass::WallThumbnail));
    let page = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert!(page.items[0].wall_thumbnail.is_some());
}

async fn published_derivative_kinds(
    receiver: &mut tokio::sync::broadcast::Receiver<WallUpdate>,
) -> Vec<DerivativeClass> {
    let mut kinds = Vec::new();
    while kinds.len() < 2 {
        let event = recv_until(receiver, |event| {
            matches!(event, WallUpdate::DerivativesReady { derivatives, .. }
                if !derivatives.is_empty())
        })
        .await;
        if let WallUpdate::DerivativesReady { derivatives, .. } = event {
            kinds.extend(derivatives.into_iter().map(|derivative| derivative.kind));
        }
    }
    kinds
}

fn catalog_derivative_kinds(config: &AppConfig) -> Vec<String> {
    Catalog::open(&config.catalog_path())
        .unwrap()
        .all_derivatives()
        .unwrap()
        .into_iter()
        .map(|record| record.kind)
        .collect()
}

async fn scanned_asset(fixture: &ProgressiveFixture, service: &AppService) -> String {
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .next()
        .unwrap()
        .id
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_preview_generates_and_publishes_thumbnail_before_preview() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let asset_id = scanned_asset(&fixture, &service).await;
    let mut updates = service.subscribe_wall_updates();

    service
        .request_derivatives(DerivativeRequest::visible_screen_preview(vec![asset_id]))
        .await
        .unwrap();

    assert_eq!(
        published_derivative_kinds(&mut updates).await,
        vec![
            DerivativeClass::WallThumbnail,
            DerivativeClass::ScreenPreview
        ]
    );
    let catalog_kinds = catalog_derivative_kinds(&fixture.config);
    assert_eq!(catalog_kinds.len(), 2);
    assert!(catalog_kinds.iter().any(|kind| kind == "wall_thumbnail"));
    assert!(catalog_kinds.iter().any(|kind| kind == "screen_preview"));
}

async fn prepare_wall_ready_fixture() -> (ProgressiveFixture, AppService, String) {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let asset_id = scanned_asset(&fixture, &service).await;
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    (fixture, service, asset_id)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_folders_preserves_captured_derivative_work_and_services_new_foreground() {
    let (fixture, service, asset_a) = prepare_wall_ready_fixture().await;
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    let starts = Arc::new(AtomicUsize::new(0));
    let (background, _, release) =
        begin_blocked_background_preview(&service, &asset_a, starts.clone()).await;
    let other = fixture.temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    ImageBuffer::from_pixel(12, 8, Rgb([10_u8, 20, 30]))
        .save(other.join("other.jpg"))
        .unwrap();
    let mut updates = service.subscribe_wall_updates();
    let b = service
        .start_scan(&other)
        .await
        .unwrap()
        .active_source
        .unwrap()
        .selection_id;
    recv_until(&mut updates, |event| matches!(event, WallUpdate::MetadataSettled { selection_id, .. } if selection_id == &b)).await;
    let asset_b = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    tokio::time::timeout(
        Duration::from_secs(2),
        service.request_derivatives(DerivativeRequest::visible(vec![asset_b])),
    )
    .await
    .expect("B visible work must not wait for A")
    .unwrap();
    release.notify_waiters();
    background
        .await
        .unwrap()
        .expect("switching view must retain A's captured derivative work");
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    assert!(
        catalog
            .all_derivatives()
            .unwrap()
            .iter()
            .any(|record| record.asset_id.as_uuid().to_string() == asset_a
                && record.kind == "screen_preview")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_folders_promotes_visible_work_past_old_visible_queue() {
    let fixture = ProgressiveFixture::new(2);
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let other = fixture.temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    ImageBuffer::from_pixel(12, 8, Rgb([10_u8, 20, 30]))
        .save(other.join("other.jpg"))
        .unwrap();
    let mut updates = service.subscribe_wall_updates();
    let b = service.start_scan(&other).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_b = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let assets_a: Vec<_> = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_gate(
            assets_a[0].clone(),
            DerivativeClass::WallThumbnail,
            entered.clone(),
            release.clone(),
        )
        .await
        .unwrap();
    let background_service = service.clone();
    let background = tokio::spawn(async move {
        background_service
            .request_derivatives(DerivativeRequest::visible(assets_a))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), entered.notified())
        .await
        .unwrap();
    service
        .activate_saved_folder(b.saved_folders.active_entry_id.as_ref().unwrap())
        .await
        .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    let deadline_reached = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let deadline_flag = deadline_reached.clone();
    let deadline_release = release.clone();
    // Use an independent deadline so a scheduler busy-loop cannot starve the timer.
    let deadline = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        deadline_flag.store(true, Ordering::SeqCst);
        deadline_release.notify_waiters();
    });
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_b]))
        .await
        .unwrap();
    let completed_before_release = !deadline_reached.load(Ordering::SeqCst);
    release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(2), background)
        .await
        .expect("old Visible batch must complete after release")
        .unwrap()
        .unwrap();
    deadline.join().unwrap();
    assert!(
        completed_before_release,
        "new foreground Visible work must bypass the old folder's queued Visible work"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_folders_removing_background_folder_cancels_its_unadmitted_derivative() {
    let (fixture, service, asset_a) = prepare_wall_ready_fixture().await;
    let entry_a = service
        .bootstrap()
        .unwrap()
        .saved_folders
        .active_entry_id
        .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    let (background, _, release) =
        begin_blocked_background_preview(&service, &asset_a, Arc::new(AtomicUsize::new(0))).await;
    let other = fixture.temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    service.start_scan(&other).await.unwrap();
    service.remove_saved_folder(&entry_a).unwrap();
    release.notify_waiters();
    assert!(
        background.await.unwrap().is_err(),
        "removed background folder must not commit queued work"
    );
    assert!(
        !Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .all_derivatives()
            .unwrap()
            .iter()
            .any(|record| record.asset_id.as_uuid().to_string() == asset_a
                && record.kind == "screen_preview")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_folders_return_syncs_scope_for_visible_child_derivatives() {
    let fixture = ProgressiveFixture::new(1);
    let child = fixture.source.join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::copy(
        fixture.source.join("photo-000.jpg"),
        child.join("child.jpg"),
    )
    .unwrap();
    let other = fixture.temp.path().join("other");
    std::fs::create_dir(&other).unwrap();
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service
        .update_gallery_scope(GalleryScope::CurrentFolder)
        .await
        .unwrap();
    let mut updates = service.subscribe_wall_updates();
    let a = service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    assert_eq!(
        service
            .query_wall(query(SortDirection::OldestFirst))
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    service.start_scan(&other).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    service
        .update_gallery_scope(GalleryScope::IncludeSubfolders)
        .await
        .unwrap();
    service
        .activate_saved_folder(a.saved_folders.active_entry_id.as_ref().unwrap())
        .await
        .unwrap();
    let wall = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(wall.items.len(), 2);
    let child = wall
        .items
        .into_iter()
        .find(|asset| asset.display_name == "child.jpg")
        .unwrap();
    service
        .request_derivatives(DerivativeRequest::visible(vec![child.id]))
        .await
        .expect("returning A must use the current recursive scope for the visible child");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn switching_folders_overlapping_parent_and_child_retain_collection_membership() {
    let fixture = ProgressiveFixture::new(2);
    let child = fixture.source.join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::copy(
        fixture.source.join("photo-000.jpg"),
        child.join("child.jpg"),
    )
    .unwrap();
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    let parent = service.start_scan(&fixture.source).await.unwrap();
    let parent_group = FolderGroupId::from_uuid(
        uuid::Uuid::parse_str(&parent.saved_folders.entries[0].folder_id).unwrap(),
    );
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::CatalogBatch { .. })
    })
    .await;
    service.start_scan(&child).await.unwrap();
    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while catalog
            .wall_preview_counts_scoped(parent_group, GalleryScope::IncludeSubfolders)
            .unwrap()
            .screen_ready
            != 3
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("parent collection must retain the child asset after child's concurrent scan");
    let mut catalog = catalog;
    let parent_record = catalog.folder_group(parent_group).unwrap().unwrap();
    // Restore the cached parent selection without admitting a new scan that
    // could overwrite the asset's legacy primary-group field.
    catalog
        .set_active_selection(Some(&photo_catalog::StoredSourceSelection {
            library_id: parent_record.library_id,
            relative_folder: parent_record.relative_path,
        }))
        .unwrap();
    let child_asset = catalog
        .assets(parent_record.library_id)
        .unwrap()
        .into_iter()
        .find(|asset| asset.display_path == "child/child.jpg")
        .unwrap();
    let screen = catalog
        .all_derivatives()
        .unwrap()
        .into_iter()
        .find(|record| record.asset_id == child_asset.id && record.kind == "screen_preview")
        .unwrap();
    assert!(
        !service
            .read_derivative(
                &child_asset.id.as_uuid().to_string(),
                DerivativeClass::ScreenPreview,
                &screen.cache_key
            )
            .expect("shared cached derivative must remain readable from its parent membership")
            .is_empty()
    );
}

async fn prepare_managed_cache_wall_ready_fixture() -> (ProgressiveFixture, AppService, String) {
    let fixture = ProgressiveFixture::new(1);
    let mut service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    service.set_derivative_cache_budget_for_test(CacheBudget::from_total_space(1_000_000));
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let asset_id = scanned_asset(&fixture, &service).await;
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    (fixture, service, asset_id)
}

async fn prepare_unready_fixture() -> (ProgressiveFixture, AppService, String) {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let asset_id = scanned_asset(&fixture, &service).await;
    (fixture, service, asset_id)
}

async fn begin_blocked_background_preview(
    service: &AppService,
    asset_id: &str,
    starts: Arc<AtomicUsize>,
) -> (
    tokio::task::JoinHandle<Result<(), photo_app_service::AppServiceError>>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
) {
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::ScreenPreview,
            entered.clone(),
            release.clone(),
            starts,
        )
        .await;
    let background_service = service.clone();
    let background_asset = asset_id.to_owned();
    let background = tokio::spawn(async move {
        background_service
            .request_derivatives(DerivativeRequest {
                asset_ids: vec![background_asset],
                priority: photo_app_service::DerivativePriority::NearViewport,
                kind: DerivativeClass::ScreenPreview,
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), entered.notified())
        .await
        .expect("background preview should reach its deterministic gate");
    (background, entered, release)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn viewer_request_restarts_once_after_colliding_background_was_invalidated() {
    let (fixture, service, asset_id) = prepare_wall_ready_fixture().await;
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    let starts = Arc::new(AtomicUsize::new(0));
    let commits = Arc::new(AtomicUsize::new(0));
    service
        .install_screen_preview_commit_test_counter(commits.clone())
        .await;
    let (background, _entered, release) =
        begin_blocked_background_preview(&service, &asset_id, starts.clone()).await;

    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    let foreground_service = service.clone();
    let foreground_asset = asset_id.clone();
    let foreground = tokio::spawn(async move {
        foreground_service
            .request_derivatives(DerivativeRequest::visible_screen_preview(vec![
                foreground_asset,
            ]))
            .await
    });

    release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if starts.load(Ordering::SeqCst) >= 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("foreground request should receive a replacement attempt");
    release.notify_waiters();
    assert!(foreground.await.unwrap().is_ok());
    assert!(background.await.unwrap().is_err());
    assert_eq!(starts.load(Ordering::SeqCst), 2);
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert_eq!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .all_derivatives()
            .unwrap()
            .into_iter()
            .filter(|record| record.kind == "screen_preview")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn viewer_request_promotes_current_background_encode() {
    let (fixture, service, asset_id) = prepare_wall_ready_fixture().await;
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    let starts = Arc::new(AtomicUsize::new(0));
    let commits = Arc::new(AtomicUsize::new(0));
    service
        .install_screen_preview_commit_test_counter(commits.clone())
        .await;
    let (background, _entered, release) =
        begin_blocked_background_preview(&service, &asset_id, starts.clone()).await;
    let foreground_service = service.clone();
    let foreground_asset = asset_id.clone();
    let foreground = tokio::spawn(async move {
        foreground_service
            .request_derivatives(DerivativeRequest::visible_screen_preview(vec![
                foreground_asset,
            ]))
            .await
    });

    release.notify_waiters();
    assert!(foreground.await.unwrap().is_ok());
    assert!(background.await.unwrap().is_ok());
    assert_eq!(starts.load(Ordering::SeqCst), 1);
    assert_eq!(commits.load(Ordering::SeqCst), 1);
    assert_eq!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .all_derivatives()
            .unwrap()
            .into_iter()
            .filter(|record| record.kind == "screen_preview")
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn direct_video_derivative_request_is_rejected_without_work() {
    let fixture = ProgressiveFixture::video_only();
    let reader = CountingReader::default();
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(reader.clone())).unwrap();
    let mut updates = service.subscribe_wall_updates();
    let bootstrap = service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let library_id = LibraryId::from_uuid(
        uuid::Uuid::parse_str(&bootstrap.active_source.as_ref().unwrap().id).unwrap(),
    );
    let video_id = AssetId::for_path(
        library_id,
        &RelativePathKey::from_relative_path(Path::new("clip.mp4")).unwrap(),
    );
    let reads_before = reader.count();
    let cache_before = managed_cache_files(fixture.config.cache_dir());
    let catalog_before = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .all_derivatives()
        .unwrap();

    for request in [
        DerivativeRequest::visible(vec![video_id.as_uuid().hyphenated().to_string()]),
        DerivativeRequest::visible_screen_preview(vec![
            video_id.as_uuid().hyphenated().to_string(),
        ]),
    ] {
        assert!(matches!(
            service.request_derivatives(request).await,
            Err(photo_app_service::AppServiceError::DerivativeUnavailable)
        ));
    }
    assert_eq!(reader.count(), reads_before);
    assert_eq!(
        managed_cache_files(fixture.config.cache_dir()),
        cache_before
    );
    assert_eq!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .all_derivatives()
            .unwrap(),
        catalog_before
    );
    while let Ok(event) = updates.try_recv() {
        assert!(!matches!(event, WallUpdate::DerivativesReady { .. }));
    }
}

async fn assert_compiled_photo_capability(expect_heif: bool) {
    let fixture = ProgressiveFixture::capability_matrix();
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let bootstrap = service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;

    let library_id =
        LibraryId::from_uuid(uuid::Uuid::parse_str(&bootstrap.active_source.unwrap().id).unwrap());
    let jpeg_id = AssetId::for_path(
        library_id,
        &RelativePathKey::from_relative_path(Path::new("displayable.jpg")).unwrap(),
    );
    let heif_id = AssetId::for_path(
        library_id,
        &RelativePathKey::from_relative_path(Path::new("phone.HEIC")).unwrap(),
    );
    let page = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    let expected_names = if expect_heif {
        vec!["displayable.jpg", "phone.HEIC"]
    } else {
        vec!["displayable.jpg"]
    };
    let mut actual_names = page
        .items
        .iter()
        .map(|asset| asset.display_name.as_str())
        .collect::<Vec<_>>();
    actual_names.sort_unstable();
    assert_eq!(actual_names, expected_names);
    assert_eq!(page.indexed_count, expected_names.len() as u64);
    assert_eq!(page.total_count, expected_names.len() as u64);

    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    assert_eq!(
        catalog.find_asset(heif_id).unwrap().unwrap().media_kind,
        MediaKind::Heif,
        "HEIF inventory must survive a decoder-disabled build"
    );
    drop(catalog);

    let heif_text = heif_id.as_uuid().hyphenated().to_string();
    if expect_heif {
        let ids = vec![
            jpeg_id.as_uuid().hyphenated().to_string(),
            heif_text.clone(),
        ];
        service
            .request_derivatives(DerivativeRequest::visible(ids.clone()))
            .await
            .unwrap();
        assert_eq!(
            recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 2)
                .await
                .len(),
            2
        );
        service
            .request_derivatives(DerivativeRequest::visible_screen_preview(ids))
            .await
            .unwrap();
        assert_eq!(
            recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 2)
                .await
                .len(),
            2
        );
    } else {
        for request in [
            DerivativeRequest::visible(vec![heif_text.clone()]),
            DerivativeRequest::visible_screen_preview(vec![heif_text]),
        ] {
            assert!(matches!(
                service.request_derivatives(request).await,
                Err(photo_app_service::AppServiceError::DerivativeUnavailable)
            ));
        }
        let jpeg_text = jpeg_id.as_uuid().hyphenated().to_string();
        service
            .request_derivatives(DerivativeRequest::visible(vec![jpeg_text.clone()]))
            .await
            .unwrap();
        assert_eq!(
            recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 1)
                .await
                .len(),
            1
        );
        service
            .request_derivatives(DerivativeRequest::visible_screen_preview(vec![jpeg_text]))
            .await
            .unwrap();
        assert_eq!(
            recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 1)
                .await
                .len(),
            1
        );
    }

    service.wait_for_derivative_tasks_quiescent_test().await;
    let derivatives = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .all_derivatives()
        .unwrap();
    assert!(derivatives.iter().all(|record| {
        record.asset_id == jpeg_id || (expect_heif && record.asset_id == heif_id)
    }));
    assert_eq!(
        derivatives
            .iter()
            .filter(|record| record.kind == "wall_thumbnail")
            .count(),
        expected_names.len()
    );
    assert_eq!(
        derivatives
            .iter()
            .filter(|record| record.kind == "screen_preview")
            .count(),
        expected_names.len()
    );
}

#[cfg(feature = "heic")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn enabled_build_presents_and_generates_derivatives_for_heif() {
    assert_compiled_photo_capability(true).await;
}

#[cfg(feature = "heic")]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn corrupt_heif_is_isolated_while_healthy_jpeg_and_heif_finish_without_source_changes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    ImageBuffer::from_pixel(16, 12, Rgb([20_u8, 40, 60]))
        .save(source.join("healthy.jpg"))
        .unwrap();
    std::fs::copy(
        heif_fixture("iphone-8bit.heic"),
        source.join("healthy.heic"),
    )
    .unwrap();
    std::fs::copy(heif_fixture("truncated.heic"), source.join("corrupt.heic")).unwrap();
    let source_before = source_file_snapshots(&source);
    assert_eq!(
        source_before
            .iter()
            .map(|file| file.name.to_string_lossy().into_owned())
            .collect::<Vec<_>>(),
        ["corrupt.heic", "healthy.heic", "healthy.jpg"]
    );

    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let service = AppService::open_with_reader(
        config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let bootstrap = service.start_scan(&source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    wait_for_scan_cleanup(&service).await;

    let library_id =
        LibraryId::from_uuid(uuid::Uuid::parse_str(&bootstrap.active_source.unwrap().id).unwrap());
    let asset_id = |name: &str| {
        AssetId::for_path(
            library_id,
            &RelativePathKey::from_relative_path(Path::new(name)).unwrap(),
        )
    };
    let corrupt_id = asset_id("corrupt.heic");
    let healthy_ids = [asset_id("healthy.heic"), asset_id("healthy.jpg")];
    let all_ids = [corrupt_id, healthy_ids[0], healthy_ids[1]]
        .into_iter()
        .map(|id| id.as_uuid().hyphenated().to_string())
        .collect::<Vec<_>>();

    let failure = tokio::time::timeout(
        Duration::from_secs(10),
        service.request_derivatives(DerivativeRequest::visible(all_ids)),
    )
    .await
    .expect("corrupt HEIF must not stall the derivative batch")
    .expect_err("the corrupt HEIF must fail its wall derivative");
    assert_eq!(
        failure.to_string(),
        "one or more requested previews could not be generated"
    );
    let corrupt_text = corrupt_id.as_uuid().hyphenated().to_string();
    let warning = recv_until(&mut updates, |event| {
        matches!(
            event,
            WallUpdate::Warning {
                asset_id: Some(id),
                warning: WallWarningState {
                    code,
                    retryable: false,
                },
                ..
            } if id == &corrupt_text && code == "wallThumbnailUnavailable"
        )
    })
    .await;
    assert!(matches!(
        warning,
        WallUpdate::Warning {
            asset_id: Some(id),
            warning: WallWarningState { code, retryable: false },
            ..
        } if id == corrupt_text && code == "wallThumbnailUnavailable"
    ));

    let healthy_text = healthy_ids
        .iter()
        .map(|id| id.as_uuid().hyphenated().to_string())
        .collect::<Vec<_>>();
    tokio::time::timeout(
        Duration::from_secs(10),
        service.request_derivatives(DerivativeRequest::visible_screen_preview(healthy_text)),
    )
    .await
    .expect("healthy screen derivatives must not stall behind corrupt HEIF")
    .unwrap();
    service.wait_for_derivative_tasks_quiescent_test().await;

    let page = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    let mut names = page
        .items
        .iter()
        .map(|asset| asset.display_name.as_str())
        .collect::<Vec<_>>();
    names.sort_unstable();
    assert_eq!(names, ["corrupt.heic", "healthy.heic", "healthy.jpg"]);
    assert_eq!(page.total_count, 3);
    assert_eq!(page.preview_counts.wall_failed, 1);
    let corrupt_asset = page
        .items
        .iter()
        .find(|asset| asset.display_name == "corrupt.heic")
        .unwrap();
    assert!(corrupt_asset.wall_thumbnail.is_none());
    assert!(corrupt_asset.screen_preview.is_none());
    assert_eq!(
        corrupt_asset
            .warning
            .as_ref()
            .map(|warning| warning.code.as_str()),
        Some("derivativeUnavailable")
    );
    assert_eq!(
        corrupt_asset
            .warning
            .as_ref()
            .map(|warning| warning.retryable),
        Some(false)
    );
    for asset in page
        .items
        .iter()
        .filter(|asset| asset.display_name != "corrupt.heic")
    {
        for reference in [
            asset.wall_thumbnail.as_ref().unwrap(),
            asset.screen_preview.as_ref().unwrap(),
        ] {
            let bytes = service
                .read_derivative(&reference.asset_id, reference.kind, &reference.key)
                .unwrap();
            assert_eq!(
                image::guess_format(&bytes).unwrap(),
                image::ImageFormat::Jpeg
            );
        }
    }

    let catalog = Catalog::open(&config.catalog_path()).unwrap();
    let corrupt = catalog.find_asset(corrupt_id).unwrap().unwrap();
    let corrupt_key = wall_cache_key(&corrupt);
    let terminal = catalog
        .find_terminal_derivative_failure(
            corrupt_id,
            "wall_thumbnail",
            &corrupt_key,
            corrupt.availability,
        )
        .unwrap()
        .expect("corrupt HEIF terminal failure must persist");
    assert_eq!(terminal.failure_code, "derivative_generation_terminal");
    assert!(catalog.warning_count().unwrap() > 0);
    assert!(
        catalog
            .derivatives_for_assets(&[corrupt_id], "wall_thumbnail")
            .unwrap()
            .is_empty()
    );
    for healthy_id in healthy_ids {
        assert_eq!(
            catalog
                .derivatives_for_assets(&[healthy_id], "wall_thumbnail")
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            catalog
                .derivatives_for_assets(&[healthy_id], "screen_preview")
                .unwrap()
                .len(),
            1
        );
    }
    drop(catalog);

    assert_eq!(source_file_snapshots(&source), source_before);
}

#[cfg(not(feature = "heic"))]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_build_retains_heif_inventory_without_presenting_or_processing_it() {
    assert_compiled_photo_capability(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn legacy_screen_preview_is_repaired_locally_before_prefetching_new_preview_work() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let wall_entered = Arc::new(tokio::sync::Notify::new());
    let wall_release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::WallThumbnail,
            wall_entered.clone(),
            wall_release.clone(),
            Arc::new(AtomicUsize::new(0)),
        )
        .await;
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), wall_entered.notified())
        .await
        .expect("background wall thumbnail should be held before legacy repair");
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    let asset_id_value = uuid::Uuid::parse_str(&asset_id)
        .map(photo_domain::AssetId::from_uuid)
        .unwrap();
    let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let stored = catalog.load_app_state().unwrap().active_selection.unwrap();
    let group = catalog
        .folder_group_for_path(stored.library_id, &stored.relative_folder)
        .unwrap()
        .unwrap();
    let asset = catalog.find_asset(asset_id_value).unwrap().unwrap();
    let source_before = std::fs::read(fixture.source.join("photo-000.jpg")).unwrap();
    let source_mtime = std::fs::metadata(fixture.source.join("photo-000.jpg"))
        .unwrap()
        .modified()
        .unwrap();
    let generator = photo_cache::ImageDerivativeGenerator::new(fixture.config.cache_dir()).unwrap();
    let protected = photo_cache::ProtectedGroups::default();
    protected.protect(group).unwrap();
    generator
        .generate_screen_preview(
            &fixture.source.join("photo-000.jpg"),
            asset.id,
            asset.signature,
            asset.orientation.unwrap_or(1),
            asset.media_kind,
            group,
            &mut catalog,
            photo_cache::CacheBudget::from_total_space(10_000_000),
            &protected,
        )
        .unwrap();
    drop(catalog);

    let unavailable = fixture.temp.path().join("photos-offline");
    std::fs::rename(&fixture.source, &unavailable).unwrap();
    wall_release.notify_waiters();
    let mut first_kind = None;
    let mut screen_seen = false;
    loop {
        let event = recv_until(&mut updates, |event| {
            matches!(event, WallUpdate::DerivativesReady { .. })
        })
        .await;
        let WallUpdate::DerivativesReady { derivatives, .. } = event else {
            unreachable!();
        };
        for derivative in derivatives {
            if derivative.asset_id != asset_id {
                continue;
            }
            first_kind.get_or_insert(derivative.kind);
            if derivative.kind == DerivativeClass::ScreenPreview {
                screen_seen = true;
            }
        }
        if screen_seen {
            break;
        }
    }

    assert_eq!(first_kind, Some(DerivativeClass::WallThumbnail));
    let page = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert!(page.items[0].wall_thumbnail.is_some());
    std::fs::rename(unavailable, &fixture.source).unwrap();
    assert_eq!(
        std::fs::read(fixture.source.join("photo-000.jpg")).unwrap(),
        source_before
    );
    assert_eq!(
        std::fs::metadata(fixture.source.join("photo-000.jpg"))
            .unwrap()
            .modified()
            .unwrap(),
        source_mtime
    );
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_visible_screen_preview_does_not_fence_prefetch() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    service
        .request_derivatives(DerivativeRequest::visible_screen_preview(vec![asset_id]))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn read_derivative_returns_bytes_for_the_active_ready_asset() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    let reference = match recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { .. })
    })
    .await
    {
        WallUpdate::DerivativesReady { derivatives, .. } => derivatives[0].clone(),
        _ => unreachable!(),
    };

    let bytes = service
        .read_derivative(&reference.asset_id, reference.kind, &reference.key)
        .unwrap();
    assert_eq!(&bytes[..2], b"\xff\xd8");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn read_derivative_fails_closed_after_switching_active_source() {
    let fixture = ProgressiveFixture::new(1);
    let source_b = fixture.temp.path().join("replacement");
    std::fs::create_dir_all(&source_b).unwrap();
    ImageBuffer::from_pixel(16, 12, Rgb([31_u8, 11, 22]))
        .save(source_b.join("replacement.jpg"))
        .unwrap();
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id]))
        .await
        .unwrap();
    let reference = match recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { .. })
    })
    .await
    {
        WallUpdate::DerivativesReady { derivatives, .. } => derivatives[0].clone(),
        _ => unreachable!(),
    };

    service.start_scan(&source_b).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;

    assert!(matches!(
        service.read_derivative(&reference.asset_id, reference.kind, &reference.key),
        Err(photo_app_service::AppServiceError::ForeignAsset)
            | Err(photo_app_service::AppServiceError::UnknownAsset)
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn derivative_failure_records_and_publishes_a_terminal_asset_warning() {
    let fixture = ProgressiveFixture::new(1);
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    let original = std::fs::read(fixture.source.join("photo-000.jpg")).unwrap();
    std::fs::write(
        fixture.source.join("photo-000.jpg"),
        b"damaged after indexing",
    )
    .unwrap();

    let failure = service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(vec![
            asset_id.clone(),
        ]))
        .await
        .expect_err("failed wall generation must return an unsuccessful outcome");
    assert_eq!(
        failure.to_string(),
        "one or more requested previews could not be generated"
    );

    let warning = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::Warning { .. })
    })
    .await;
    assert!(matches!(
        warning,
        WallUpdate::Warning {
            asset_id: Some(id),
            warning: WallWarningState { retryable: false, .. },
            ..
        } if id == asset_id
    ));
    let diagnostic: String = Connection::open(fixture.config.catalog_path())
        .unwrap()
        .query_row(
            "SELECT message FROM warnings WHERE code = 'derivative_generation_terminal' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        diagnostic.contains("source image could not be decoded"),
        "terminal diagnostics must retain the decoder reason: {diagnostic}"
    );
    let page = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.total_count, 1);
    assert_eq!(page.preview_counts.wall_failed, 1);
    assert_eq!(
        page.items[0]
            .warning
            .as_ref()
            .map(|warning| warning.retryable),
        Some(false)
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(
        photo_catalog::Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .warning_count()
            .unwrap(),
        1,
        "wall and screen failures for one asset must share one bounded warning"
    );

    let mut changed = original.clone();
    changed.extend_from_slice(b"newer source signature");
    std::fs::write(fixture.source.join("photo-000.jpg"), changed).unwrap();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(vec![
            asset_id,
        ]))
        .await
        .unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives, .. } if
            derivatives.len() == 1
            && derivatives[0].kind == DerivativeClass::WallThumbnail)
    })
    .await;
    assert_eq!(
        photo_catalog::Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .warning_count()
            .unwrap(),
        0,
        "a recovered asset must stop presenting its retryable derivative warning"
    );
    assert!(
        service
            .query_wall(query(SortDirection::OldestFirst))
            .await
            .unwrap()
            .items[0]
            .warning
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_failure_retries_only_after_derivative_key_changes() {
    let fixture = ProgressiveFixture::new(1);
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    let original = std::fs::read(fixture.source.join("photo-000.jpg")).unwrap();
    std::fs::write(
        fixture.source.join("photo-000.jpg"),
        b"damaged after indexing",
    )
    .unwrap();

    let first = service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await;
    let first = first.expect_err("a failed wall request must return an unsuccessful outcome");
    assert_eq!(
        first.to_string(),
        "one or more requested previews could not be generated"
    );
    recv_until(
        &mut updates,
        |event| matches!(event, WallUpdate::Warning { asset_id: Some(id), .. } if id == &asset_id),
    )
    .await;

    let second = service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await;
    let second =
        second.expect_err("a repeated failed wall request must return an unsuccessful outcome");
    assert_eq!(
        second.to_string(),
        "one or more requested previews could not be generated"
    );
    assert!(
        updates.try_recv().is_err(),
        "the repeated failure must not publish a duplicate warning"
    );

    let mut changed = original.clone();
    changed.extend_from_slice(b"newer source signature");
    std::fs::write(fixture.source.join("photo-000.jpg"), changed).unwrap();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let pending = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(pending.preview_counts.wall_failed, 0);
    assert_eq!(pending.preview_counts.wall_ready, 0);
    assert!(
        pending.items[0]
            .warning
            .as_ref()
            .is_none_or(|warning| warning.retryable),
        "a changed source must re-enter pending work instead of retaining a terminal failure"
    );
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    let ready = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives, .. } if derivatives.iter().any(|item| item.asset_id == asset_id && item.kind == DerivativeClass::WallThumbnail))
    })
    .await;
    assert!(matches!(
        ready,
        WallUpdate::DerivativesReady {
            preview_counts: Some(photo_app_service::WallPreviewCounts {
                wall_ready: 1,
                screen_ready: 0,
                wall_failed: 0,
                screen_failed: 0,
            }),
            ..
        }
    ));
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_failure_restart_skips_same_key_and_retries_changed_key_once() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    let original = std::fs::read(fixture.source.join("photo-000.jpg")).unwrap();
    std::fs::write(
        fixture.source.join("photo-000.jpg"),
        b"damaged after indexing",
    )
    .unwrap();

    let first_attempts = Arc::new(AtomicUsize::new(0));
    let first_entered = Arc::new(tokio::sync::Notify::new());
    let first_release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::WallThumbnail,
            first_entered.clone(),
            first_release.clone(),
            first_attempts.clone(),
        )
        .await;
    let first_request_service = service.clone();
    let first_request_asset = asset_id.clone();
    let first_request = tokio::spawn(async move {
        first_request_service
            .request_derivatives(DerivativeRequest::visible(vec![first_request_asset]))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), first_entered.notified())
        .await
        .expect("initial terminal attempt should reach the gate");
    assert_eq!(first_attempts.load(Ordering::SeqCst), 1);
    first_release.notify_waiters();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), first_request)
            .await
            .expect("initial terminal request did not finish after release")
            .unwrap()
            .is_err()
    );
    recv_until(
        &mut updates,
        |event| matches!(event, WallUpdate::Warning { asset_id: Some(id), .. } if id == &asset_id),
    )
    .await;
    let failed_before_restart = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .find_asset(AssetId::from_uuid(
            uuid::Uuid::parse_str(&asset_id).unwrap(),
        ))
        .unwrap()
        .unwrap();
    let old_key = wall_cache_key(&failed_before_restart);
    assert!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .find_terminal_derivative_failure(
                failed_before_restart.id,
                "wall_thumbnail",
                &old_key,
                failed_before_restart.availability,
            )
            .unwrap()
            .is_some()
    );
    drop(service);

    let reopen_config = fixture.config.clone();
    let reopened = std::thread::spawn(move || {
        AppService::open_with_reader(
            reopen_config,
            Arc::new(photo_indexer::DefaultMetadataReader),
        )
    })
    .join()
    .unwrap()
    .unwrap();
    reopened
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let mut reopened_updates = reopened.subscribe_wall_updates();

    let same_key_attempts = Arc::new(AtomicUsize::new(0));
    let same_key_entered = Arc::new(tokio::sync::Notify::new());
    let same_key_release = Arc::new(tokio::sync::Notify::new());
    reopened
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::WallThumbnail,
            same_key_entered,
            same_key_release,
            same_key_attempts.clone(),
        )
        .await;
    assert!(
        reopened
            .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
            .await
            .is_err()
    );
    assert_eq!(same_key_attempts.load(Ordering::SeqCst), 0);

    let mut changed = original;
    changed.extend_from_slice(b"newer source signature");
    std::fs::write(fixture.source.join("photo-000.jpg"), changed).unwrap();
    reopened.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut reopened_updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let changed_asset = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .find_asset(AssetId::from_uuid(
            uuid::Uuid::parse_str(&asset_id).unwrap(),
        ))
        .unwrap()
        .unwrap();
    let changed_key = wall_cache_key(&changed_asset);
    assert_ne!(changed_key, old_key);

    let changed_attempts = Arc::new(AtomicUsize::new(0));
    let changed_entered = Arc::new(tokio::sync::Notify::new());
    let changed_release = Arc::new(tokio::sync::Notify::new());
    reopened
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::WallThumbnail,
            changed_entered.clone(),
            changed_release.clone(),
            changed_attempts.clone(),
        )
        .await;
    let changed_request_service = reopened.clone();
    let changed_request_asset = asset_id.clone();
    let changed_request = tokio::spawn(async move {
        changed_request_service
            .request_derivatives(DerivativeRequest::visible(vec![changed_request_asset]))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), changed_entered.notified())
        .await
        .expect("changed terminal attempt should reach the gate");
    assert_eq!(changed_attempts.load(Ordering::SeqCst), 1);
    changed_release.notify_waiters();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), changed_request)
            .await
            .expect("changed terminal request did not finish after release")
            .unwrap()
            .is_ok()
    );
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    assert!(
        catalog
            .find_terminal_derivative_failure(
                changed_asset.id,
                "wall_thumbnail",
                &changed_key,
                changed_asset.availability,
            )
            .unwrap()
            .is_none()
    );
    assert!(
        catalog
            .find_terminal_derivative_failure(
                changed_asset.id,
                "wall_thumbnail",
                &old_key,
                changed_asset.availability,
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(catalog.warning_count().unwrap(), 0);
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_failure_restart_skips_same_availability_and_retries_after_recovery_once() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    let asset_uuid = AssetId::from_uuid(uuid::Uuid::parse_str(&asset_id).unwrap());
    let (library_id, old_key) = {
        let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
        let asset = catalog.find_asset(asset_uuid).unwrap().unwrap();
        (asset.library_id, wall_cache_key(&asset))
    };
    Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .mark_root_offline(library_id)
        .unwrap();

    let first_attempts = Arc::new(AtomicUsize::new(0));
    let first_entered = Arc::new(tokio::sync::Notify::new());
    let first_release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::WallThumbnail,
            first_entered,
            first_release,
            first_attempts.clone(),
        )
        .await;
    assert!(
        service
            .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
            .await
            .is_err()
    );
    assert_eq!(first_attempts.load(Ordering::SeqCst), 0);
    recv_until(&mut updates, |event| {
        matches!(
            event,
            WallUpdate::Warning {
                asset_id: Some(id),
                warning: WallWarningState { retryable: false, .. },
                ..
            } if id == &asset_id
        )
    })
    .await;
    drop(service);

    let reopen_config = fixture.config.clone();
    let reopened = std::thread::spawn(move || {
        AppService::open_with_reader(
            reopen_config,
            Arc::new(photo_indexer::DefaultMetadataReader),
        )
    })
    .join()
    .unwrap()
    .unwrap();
    reopened
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let mut reopened_updates = reopened.subscribe_wall_updates();
    let same_attempts = Arc::new(AtomicUsize::new(0));
    let same_entered = Arc::new(tokio::sync::Notify::new());
    let same_release = Arc::new(tokio::sync::Notify::new());
    reopened
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::WallThumbnail,
            same_entered,
            same_release,
            same_attempts.clone(),
        )
        .await;
    assert!(
        reopened
            .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
            .await
            .is_err()
    );
    assert_eq!(same_attempts.load(Ordering::SeqCst), 0);

    reopened.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut reopened_updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let recovered_asset = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .find_asset(asset_uuid)
        .unwrap()
        .unwrap();
    assert_eq!(wall_cache_key(&recovered_asset), old_key);
    assert_eq!(
        recovered_asset.availability,
        photo_domain::Availability::Available
    );

    let recovery_attempts = Arc::new(AtomicUsize::new(0));
    let recovery_entered = Arc::new(tokio::sync::Notify::new());
    let recovery_release = Arc::new(tokio::sync::Notify::new());
    reopened
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::WallThumbnail,
            recovery_entered.clone(),
            recovery_release.clone(),
            recovery_attempts.clone(),
        )
        .await;
    let recovery_service = reopened.clone();
    let recovery_request = tokio::spawn(async move {
        recovery_service
            .request_derivatives(DerivativeRequest::visible(vec![asset_id]))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), recovery_entered.notified())
        .await
        .expect("availability recovery should reach one wall attempt");
    assert_eq!(recovery_attempts.load(Ordering::SeqCst), 1);
    recovery_release.notify_waiters();
    assert!(
        tokio::time::timeout(Duration::from_secs(5), recovery_request)
            .await
            .expect("availability recovery request did not finish")
            .unwrap()
            .is_ok()
    );
    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    assert!(
        catalog
            .find_terminal_derivative_failure(
                asset_uuid,
                "wall_thumbnail",
                &old_key,
                photo_domain::Availability::RootOffline,
            )
            .unwrap()
            .is_none()
    );
    assert_eq!(catalog.warning_count().unwrap(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn online_reopen_prefetches_the_full_group_when_only_wall_rows_are_durable() {
    let fixture = ProgressiveFixture::new(2);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();
    service
        .request_derivatives(DerivativeRequest::visible(ids.clone()))
        .await
        .unwrap();
    let _ = recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, ids.len()).await;
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let screen_ids = catalog
        .all_derivatives()
        .unwrap()
        .into_iter()
        .filter(|record| record.kind == "screen_preview")
        .map(|record| record.id)
        .collect::<Vec<_>>();
    catalog.delete_derivatives(&screen_ids).unwrap();
    assert!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .all_derivatives()
            .unwrap()
            .into_iter()
            .all(|record| record.kind == "wall_thumbnail")
    );
    drop(service);

    let reopened = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut reopened_updates = reopened.subscribe_wall_updates();
    let page = reopened
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert!(
        page.items
            .iter()
            .all(|asset| asset.wall_thumbnail.is_some())
    );
    assert!(
        page.items
            .iter()
            .all(|asset| asset.screen_preview.is_none())
    );
    let screen = recv_derivatives_until(
        &mut reopened_updates,
        DerivativeClass::ScreenPreview,
        ids.len(),
    )
    .await;
    assert_eq!(
        screen
            .iter()
            .map(|reference| reference.asset_id.as_str())
            .collect::<std::collections::HashSet<_>>(),
        ids.iter().map(String::as_str).collect()
    );
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_photo_page_reaches_wall_outcome_before_background_screens_begin() {
    let fixture = ProgressiveFixture::new(301);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service.start_scan(&fixture.source).await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let page = service
                .query_wall(WallQueryRequest {
                    cursor: None,
                    limit: 1,
                    direction: SortDirection::OldestFirst,
                })
                .await
                .unwrap();
            if page.order_state == OrderState::Settled {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the large fixture should settle before prefetch assertions");
    let mut updates = service.subscribe_wall_updates();

    let first_page = service
        .query_wall(WallQueryRequest {
            cursor: None,
            limit: 250,
            direction: SortDirection::OldestFirst,
        })
        .await
        .unwrap();
    let second_page = service
        .query_wall(WallQueryRequest {
            cursor: first_page.next_cursor.clone(),
            limit: 250,
            direction: SortDirection::OldestFirst,
        })
        .await
        .unwrap();
    let late_asset = second_page.items.first().unwrap().id.clone();
    let total_assets = first_page.items.len() + second_page.items.len();

    #[derive(Default)]
    struct PublicationObservation {
        wall_count: usize,
        screen_count: usize,
        wall_count_before_first_screen: Option<usize>,
    }
    let observation = Arc::new(Mutex::new(PublicationObservation::default()));
    let observed = observation.clone();
    let collector = tokio::spawn(async move {
        loop {
            let Ok(event) = updates.recv().await else {
                return;
            };
            let WallUpdate::DerivativesReady { derivatives, .. } = event else {
                continue;
            };
            let mut observation = observed.lock().unwrap();
            for derivative in derivatives {
                match derivative.kind {
                    DerivativeClass::WallThumbnail => observation.wall_count += 1,
                    DerivativeClass::ScreenPreview => {
                        if observation.wall_count_before_first_screen.is_none() {
                            observation.wall_count_before_first_screen =
                                Some(observation.wall_count);
                        }
                        observation.screen_count += 1;
                    }
                }
            }
        }
    });

    let wall_entered = Arc::new(tokio::sync::Notify::new());
    let wall_release = Arc::new(tokio::sync::Notify::new());
    let screen_encode_starts = Arc::new(AtomicUsize::new(0));
    service
        .install_screen_preview_encode_test_counter(screen_encode_starts.clone())
        .await;
    service
        .install_derivative_test_gate(
            late_asset,
            DerivativeClass::WallThumbnail,
            wall_entered.clone(),
            wall_release.clone(),
        )
        .await
        .unwrap();

    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    tokio::time::timeout(Duration::from_secs(5), wall_entered.notified())
        .await
        .expect("the late-page wall thumbnail should be reached");

    tokio::time::sleep(Duration::from_millis(100)).await;
    {
        let observation_at_hold = observation.lock().unwrap();
        assert_eq!(
            observation_at_hold.screen_count, 0,
            "screen previews must wait for every wall page while the late page is blocked"
        );
        assert!(
            observation_at_hold.wall_count < total_assets,
            "the gated late-page wall thumbnail must not be published while blocked"
        );
    }
    assert_eq!(
        screen_encode_starts.load(Ordering::SeqCst),
        0,
        "screen encoding must not start while a late wall page is blocked"
    );
    let screen_rows = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .all_derivatives()
        .unwrap()
        .into_iter()
        .filter(|record| record.kind == "screen_preview")
        .count();
    assert_eq!(
        screen_rows, 0,
        "no screen preview may commit before full wall coverage"
    );

    wall_release.notify_waiters();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if observation
                .lock()
                .unwrap()
                .wall_count_before_first_screen
                .is_some()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("screen previews should resume after full wall coverage");
    assert!(screen_encode_starts.load(Ordering::SeqCst) > 0);
    let observation_after_release = observation.lock().unwrap();
    assert_eq!(
        observation_after_release.wall_count_before_first_screen,
        Some(total_assets),
        "all wall pages must be published before the first screen preview"
    );
    assert!(observation_after_release.screen_count > 0);
    drop(observation_after_release);
    collector.abort();
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stale_screen_preview_is_discarded_after_post_encode_invalidation() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .next()
        .unwrap()
        .id;

    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    let source_before = std::fs::read(fixture.source.join("photo-000.jpg")).unwrap();
    let source_modified_before = std::fs::metadata(fixture.source.join("photo-000.jpg"))
        .unwrap()
        .modified()
        .unwrap();
    let cache_before = managed_cache_files(fixture.config.cache_dir());
    let post_encode_entered = Arc::new(tokio::sync::Notify::new());
    let post_encode_release = Arc::new(tokio::sync::Notify::new());
    service
        .install_screen_preview_post_encode_test_gate(
            post_encode_entered.clone(),
            post_encode_release.clone(),
        )
        .await;

    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    tokio::time::timeout(Duration::from_secs(5), post_encode_entered.notified())
        .await
        .expect("screen preview should reach the post-encode boundary");

    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    post_encode_release.notify_waiters();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let records = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .all_derivatives()
        .unwrap();
    assert!(records.iter().all(|record| record.kind != "screen_preview"));
    assert_eq!(
        managed_cache_files(fixture.config.cache_dir()),
        cache_before,
        "stale screen work must not write or evict managed cache files"
    );
    assert_eq!(
        std::fs::read(fixture.source.join("photo-000.jpg")).unwrap(),
        source_before
    );
    assert_eq!(
        std::fs::metadata(fixture.source.join("photo-000.jpg"))
            .unwrap()
            .modified()
            .unwrap(),
        source_modified_before
    );
    while let Ok(event) = updates.try_recv() {
        assert!(!matches!(
            event,
            WallUpdate::DerivativesReady { derivatives, .. }
                if derivatives.iter().any(|item| item.kind == DerivativeClass::ScreenPreview)
        ));
    }
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn invalidation_before_commit_admission_has_no_side_effects() {
    for _ in 0..20 {
        let (fixture, mut service, asset_id) = prepare_wall_ready_fixture().await;
        seed_evictable_screen_preview_and_warning(&fixture, &asset_id);
        service.set_derivative_cache_budget_for_test(CacheBudget::from_total_space(1_000_000));
        let before = snapshot_cache_catalog_warnings(&fixture.config, &asset_id);
        let mut updates = service.subscribe_wall_updates();

        let post_encode_entered = Arc::new(tokio::sync::Notify::new());
        let post_encode_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_screen_preview_post_encode_test_gate(
                post_encode_entered.clone(),
                post_encode_release.clone(),
            )
            .await;
        let commits = Arc::new(AtomicUsize::new(0));
        service
            .install_screen_preview_commit_test_counter(commits.clone())
            .await;
        let completed = Arc::new(tokio::sync::Notify::new());
        service
            .install_derivative_completion_test_hook(completed.clone())
            .await;

        // Keep automatic prefetch from satisfying the encode gate before the
        // explicit request below has registered its completion waiter.
        let collection_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_collection_enqueue_test_gate(
                Arc::new(tokio::sync::Notify::new()),
                collection_release.clone(),
            )
            .await;
        service
            .set_interaction(photo_app_service::InteractionState::Idle)
            .await;
        let background_service = service.clone();
        let background_asset = asset_id.clone();
        let background = tokio::spawn(async move {
            background_service
                .request_derivatives(DerivativeRequest {
                    asset_ids: vec![background_asset],
                    priority: photo_app_service::DerivativePriority::NearViewport,
                    kind: DerivativeClass::ScreenPreview,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), post_encode_entered.notified())
            .await
            .expect("screen preview should reach the pre-admission gate");

        service
            .set_interaction(photo_app_service::InteractionState::Active)
            .await;
        service
            .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
            .await
            .unwrap();
        post_encode_release.notify_waiters();
        collection_release.notify_one();
        assert!(
            tokio::time::timeout(Duration::from_secs(5), background)
                .await
                .expect("the invalidated request should resolve")
                .unwrap()
                .is_err()
        );
        tokio::time::timeout(Duration::from_secs(5), completed.notified())
            .await
            .expect("stale worker should finish after admission is rejected");

        let after = snapshot_cache_catalog_warnings(&fixture.config, &asset_id);
        assert_eq!(after, before);
        assert_eq!(commits.load(Ordering::SeqCst), 0);
        assert_eq!(derivative_update_counts(&mut updates), (0, 0, 0));
    }
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn scope_update_returns_bootstrap_refreshed_after_invalidation_waits() {
    let (_fixture, service, asset_id) = prepare_managed_cache_wall_ready_fixture().await;
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    let post_admission_entered = Arc::new(tokio::sync::Notify::new());
    let post_admission_release = Arc::new(tokio::sync::Notify::new());
    service
        .install_screen_preview_post_admission_test_gate(
            post_admission_entered.clone(),
            post_admission_release.clone(),
        )
        .await;
    let invalidation_waiting = Arc::new(tokio::sync::Notify::new());
    service
        .install_background_invalidation_wait_test_hook(invalidation_waiting.clone())
        .await;

    let background_service = service.clone();
    let background = tokio::spawn(async move {
        background_service
            .request_derivatives(DerivativeRequest {
                asset_ids: vec![asset_id],
                priority: photo_app_service::DerivativePriority::NearViewport,
                kind: DerivativeClass::ScreenPreview,
            })
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), post_admission_entered.notified())
        .await
        .expect("screen preview should hold after commit admission");

    let scope_service = service.clone();
    let scope_update = tokio::spawn(async move {
        scope_service
            .update_gallery_scope(GalleryScope::CurrentFolder)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), invalidation_waiting.notified())
        .await
        .expect("scope update should wait for the admitted background commit");
    let duplicate_service = service.clone();
    let mut duplicate_update = tokio::spawn(async move {
        duplicate_service
            .update_gallery_scope(GalleryScope::CurrentFolder)
            .await
    });
    assert!(
        tokio::time::timeout(Duration::from_millis(50), &mut duplicate_update)
            .await
            .is_err(),
        "an identical scope update must wait for the in-flight refresh"
    );
    service.update_appearance(Appearance::Dark).unwrap();

    post_admission_release.notify_waiters();
    assert!(background.await.unwrap().is_ok());
    let returned = scope_update.await.unwrap().unwrap();
    let duplicate_returned = duplicate_update.await.unwrap().unwrap();

    assert_eq!(returned.settings.appearance, Appearance::Dark);
    assert_eq!(duplicate_returned.settings.appearance, Appearance::Dark);
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitted_commit_finishes_before_later_invalidation() {
    for _ in 0..20 {
        let (fixture, mut service, asset_id) = prepare_managed_cache_wall_ready_fixture().await;
        seed_evictable_screen_preview_and_warning(&fixture, &asset_id);
        service.set_derivative_cache_budget_for_test(CacheBudget::from_total_space(1_000_000));
        let expected_selection_id = service.active_selection_id().unwrap();
        let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
        let active_selection = catalog.load_app_state().unwrap().active_selection.unwrap();
        let expected_group = catalog
            .folder_group_for_path(
                active_selection.library_id,
                &active_selection.relative_folder,
            )
            .unwrap()
            .unwrap();
        let expected_source_id = active_selection
            .library_id
            .as_uuid()
            .hyphenated()
            .to_string();
        let mut updates = service.subscribe_wall_updates();
        let commits = Arc::new(AtomicUsize::new(0));
        service
            .install_screen_preview_commit_test_counter(commits.clone())
            .await;

        let pre_admission_entered = Arc::new(tokio::sync::Notify::new());
        let pre_admission_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_screen_preview_post_encode_test_gate(
                pre_admission_entered.clone(),
                pre_admission_release.clone(),
            )
            .await;
        let post_admission_entered = Arc::new(tokio::sync::Notify::new());
        let post_admission_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_screen_preview_post_admission_test_gate(
                post_admission_entered.clone(),
                post_admission_release.clone(),
            )
            .await;
        let completed = Arc::new(tokio::sync::Notify::new());
        service
            .install_derivative_completion_test_hook(completed.clone())
            .await;
        let invalidation_waiting = Arc::new(tokio::sync::Notify::new());
        service
            .install_background_invalidation_wait_test_hook(invalidation_waiting.clone())
            .await;
        let waiter_delivery_entered = Arc::new(tokio::sync::Notify::new());
        let waiter_delivery_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_commit_waiter_delivery_test_gate(
                waiter_delivery_entered.clone(),
                waiter_delivery_release.clone(),
            )
            .await;

        service
            .set_interaction(photo_app_service::InteractionState::Idle)
            .await;
        let background_service = service.clone();
        let background_asset = asset_id.clone();
        let mut background = tokio::spawn(async move {
            background_service
                .request_derivatives(DerivativeRequest {
                    asset_ids: vec![background_asset],
                    priority: photo_app_service::DerivativePriority::NearViewport,
                    kind: DerivativeClass::ScreenPreview,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), pre_admission_entered.notified())
            .await
            .expect("screen preview should reach the pre-admission gate");
        pre_admission_release.notify_waiters();
        tokio::time::timeout(Duration::from_secs(5), post_admission_entered.notified())
            .await
            .expect("screen preview should reach the post-admission gate");

        service
            .set_interaction(photo_app_service::InteractionState::Active)
            .await;
        let visible_service = service.clone();
        let visible_asset = asset_id.clone();
        let visible = tokio::spawn(async move {
            visible_service
                .request_derivatives(DerivativeRequest::visible(vec![visible_asset]))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), invalidation_waiting.notified())
            .await
            .expect("visible invalidation should wait on the admitted commit");
        assert!(!visible.is_finished());

        post_admission_release.notify_waiters();
        tokio::time::timeout(Duration::from_secs(5), waiter_delivery_entered.notified())
            .await
            .expect("admitted worker should deliver its waiter before completion notification");
        let background_result = tokio::time::timeout(Duration::from_secs(5), &mut background)
            .await
            .expect("background waiter should resolve before invalidation is released")
            .expect("background request task panicked");
        assert!(background_result.is_ok());
        assert!(!visible.is_finished());
        waiter_delivery_release.notify_waiters();
        tokio::time::timeout(Duration::from_secs(5), completed.notified())
            .await
            .expect("admitted worker should finish its complete-commit path");
        assert!(visible.await.unwrap().is_ok());

        let snapshot = snapshot_cache_catalog_warnings(&fixture.config, &asset_id);
        assert_eq!(commits.load(Ordering::SeqCst), 1);
        let screen_rows = snapshot
            .derivatives
            .iter()
            .filter(|record| record.kind == "screen_preview")
            .collect::<Vec<_>>();
        assert_eq!(screen_rows.len(), 1);
        let screen_row = screen_rows[0];
        assert_eq!(screen_row.asset_id, asset_id);
        assert_eq!(
            screen_row.folder_group_id,
            expected_group.as_uuid().hyphenated().to_string()
        );
        assert_ne!(screen_row.cache_key, "evictable-screen-preview");
        assert!(
            snapshot.cache_files.contains(
                &fixture
                    .config
                    .cache_dir()
                    .join(&screen_row.relative_cache_path)
            )
        );
        assert!(
            !snapshot
                .cache_files
                .iter()
                .any(|path| path.ends_with("evictable/old-preview.jpg"))
        );
        assert!(snapshot.source_warnings.is_empty());
        assert!(snapshot.asset_warnings.is_empty());
        let updates_observed = derivative_update_observation(&mut updates);
        assert_eq!(updates_observed.warning_count, 0);
        assert_eq!(updates_observed.screen_publications.len(), 1);
        let (selection_id, publication) = &updates_observed.screen_publications[0];
        assert_eq!(selection_id, &expected_selection_id);
        assert_eq!(publication.asset_id, asset_id);
        assert_eq!(publication.kind, DerivativeClass::ScreenPreview);
        assert_eq!(publication.key, screen_row.cache_key);
        assert_eq!(
            updates_observed.warning_clears,
            vec![
                WarningClearObservation {
                    selection_id: expected_selection_id.clone(),
                    source_id: expected_source_id.clone(),
                    asset_id: Some(asset_id.clone()),
                    code: "derivativeUnavailable".to_owned(),
                },
                WarningClearObservation {
                    selection_id: expected_selection_id,
                    source_id: expected_source_id,
                    asset_id: None,
                    code: "screenPreviewCacheUnavailable".to_owned(),
                },
            ]
        );
    }
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitted_preview_cancellation_fails_waiters_and_releases_invalidation() {
    for _ in 0..20 {
        let (fixture, service, asset_id) = prepare_wall_ready_fixture().await;
        let post_admission_entered = Arc::new(tokio::sync::Notify::new());
        let post_admission_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_screen_preview_post_admission_test_gate(
                post_admission_entered.clone(),
                post_admission_release,
            )
            .await;
        let invalidation_waiting = Arc::new(tokio::sync::Notify::new());
        service
            .install_background_invalidation_wait_test_hook(invalidation_waiting.clone())
            .await;

        service
            .set_interaction(photo_app_service::InteractionState::Idle)
            .await;
        let background_service = service.clone();
        let background_asset = asset_id.clone();
        let mut background = tokio::spawn(async move {
            background_service
                .request_derivatives(DerivativeRequest {
                    asset_ids: vec![background_asset],
                    priority: photo_app_service::DerivativePriority::NearViewport,
                    kind: DerivativeClass::ScreenPreview,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), post_admission_entered.notified())
            .await
            .expect("screen preview should reach the admitted cancellation gate");

        service
            .set_interaction(photo_app_service::InteractionState::Active)
            .await;
        let visible_service = service.clone();
        let visible_asset = asset_id.clone();
        let visible = tokio::spawn(async move {
            visible_service
                .request_derivatives(DerivativeRequest::visible(vec![visible_asset]))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), invalidation_waiting.notified())
            .await
            .expect("visible invalidation should wait on the admitted attempt");

        service.abort_derivative_attempt_for_test().await;
        let background_result = tokio::time::timeout(Duration::from_secs(5), &mut background)
            .await
            .expect("cancellation should resolve the background waiter")
            .expect("background request task panicked");
        assert!(background_result.is_err());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), visible)
                .await
                .expect("invalidation should be released after cancellation")
                .expect("visible request task panicked")
                .is_ok()
        );
        assert_eq!(
            Catalog::open(&fixture.config.catalog_path())
                .unwrap()
                .all_derivatives()
                .unwrap()
                .into_iter()
                .filter(|record| record.kind == "screen_preview")
                .count(),
            0
        );
    }
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn admitted_preview_panic_fails_waiters_and_releases_invalidation() {
    for _ in 0..20 {
        let (fixture, service, asset_id) = prepare_wall_ready_fixture().await;
        let post_admission_entered = Arc::new(tokio::sync::Notify::new());
        let post_admission_release = Arc::new(tokio::sync::Notify::new());
        service
            .install_screen_preview_post_admission_test_gate(
                post_admission_entered.clone(),
                post_admission_release.clone(),
            )
            .await;
        let invalidation_waiting = Arc::new(tokio::sync::Notify::new());
        service
            .install_background_invalidation_wait_test_hook(invalidation_waiting.clone())
            .await;

        service
            .set_interaction(photo_app_service::InteractionState::Idle)
            .await;
        let background_service = service.clone();
        let background_asset = asset_id.clone();
        let mut background = tokio::spawn(async move {
            background_service
                .request_derivatives(DerivativeRequest {
                    asset_ids: vec![background_asset],
                    priority: photo_app_service::DerivativePriority::NearViewport,
                    kind: DerivativeClass::ScreenPreview,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), post_admission_entered.notified())
            .await
            .expect("screen preview should reach the admitted panic gate");

        service
            .set_interaction(photo_app_service::InteractionState::Active)
            .await;
        let visible_service = service.clone();
        let visible_asset = asset_id.clone();
        let visible = tokio::spawn(async move {
            visible_service
                .request_derivatives(DerivativeRequest::visible(vec![visible_asset]))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), invalidation_waiting.notified())
            .await
            .expect("visible invalidation should wait on the admitted attempt");

        service
            .install_derivative_panic_after_admission_test_hook()
            .await;
        post_admission_release.notify_waiters();
        let background_result = tokio::time::timeout(Duration::from_secs(5), &mut background)
            .await
            .expect("panic supervision should resolve the background waiter")
            .expect("background request task panicked");
        assert!(background_result.is_err());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), visible)
                .await
                .expect("invalidation should be released after panic")
                .expect("visible request task panicked")
                .is_ok()
        );
        assert_eq!(
            Catalog::open(&fixture.config.catalog_path())
                .unwrap()
                .all_derivatives()
                .unwrap()
                .into_iter()
                .filter(|record| record.kind == "screen_preview")
                .count(),
            0
        );
    }
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_screen_commit_waits_for_started_blocking_work() {
    for _ in 0..20 {
        let (fixture, service, asset_id) = prepare_wall_ready_fixture().await;
        let commit_started = Arc::new(tokio::sync::Notify::new());
        let commit_release = Arc::new(AtomicBool::new(false));
        service
            .install_managed_commit_started_test_gate(
                commit_started.clone(),
                commit_release.clone(),
            )
            .await;
        let invalidation_waiting = Arc::new(tokio::sync::Notify::new());
        service
            .install_background_invalidation_wait_test_hook(invalidation_waiting.clone())
            .await;

        service
            .set_interaction(photo_app_service::InteractionState::Idle)
            .await;
        let background_service = service.clone();
        let background_asset = asset_id.clone();
        let mut background = tokio::spawn(async move {
            background_service
                .request_derivatives(DerivativeRequest {
                    asset_ids: vec![background_asset],
                    priority: photo_app_service::DerivativePriority::NearViewport,
                    kind: DerivativeClass::ScreenPreview,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), commit_started.notified())
            .await
            .expect("screen commit should enter the blocking-work gate");

        service
            .set_interaction(photo_app_service::InteractionState::Active)
            .await;
        let visible_service = service.clone();
        let visible_asset = asset_id.clone();
        let visible = tokio::spawn(async move {
            visible_service
                .request_derivatives(DerivativeRequest::visible(vec![visible_asset]))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), invalidation_waiting.notified())
            .await
            .expect("visible invalidation should wait for started screen work");

        service.abort_derivative_attempt_for_test().await;
        tokio::task::yield_now().await;
        assert!(!background.is_finished());
        assert!(!visible.is_finished());
        commit_release.store(true, Ordering::Release);
        let background_result = tokio::time::timeout(Duration::from_secs(5), &mut background)
            .await
            .expect("supervised screen work should finish after release")
            .expect("background request task panicked");
        assert!(background_result.is_ok());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), visible)
                .await
                .expect("screen invalidation should complete after release")
                .expect("visible request task panicked")
                .is_ok()
        );
        assert_eq!(
            Catalog::open(&fixture.config.catalog_path())
                .unwrap()
                .all_derivatives()
                .unwrap()
                .into_iter()
                .filter(|record| record.kind == "screen_preview")
                .count(),
            1
        );
    }
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn shutdown_retains_an_admitted_blocking_commit_until_it_drains() {
    let (fixture, service, asset_id) = prepare_unready_fixture().await;
    let commit_started = Arc::new(tokio::sync::Notify::new());
    let commit_release = Arc::new(AtomicBool::new(false));
    service
        .install_managed_commit_started_test_gate(commit_started.clone(), commit_release.clone())
        .await;
    let request_service = service.clone();
    let request = tokio::spawn(async move {
        request_service
            .request_derivatives(DerivativeRequest::visible(vec![asset_id]))
            .await
    });
    tokio::time::timeout(Duration::from_secs(2), commit_started.notified())
        .await
        .expect("commit should be admitted and running");
    service.shutdown().await;
    assert!(
        !request.is_finished(),
        "shutdown must not settle a started blocking commit early"
    );
    commit_release.store(true, Ordering::Release);
    tokio::time::timeout(Duration::from_secs(2), request)
        .await
        .expect("admitted commit must drain after release")
        .unwrap()
        .unwrap();
    assert_eq!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .all_derivatives()
            .unwrap()
            .into_iter()
            .filter(|record| record.kind == "wall_thumbnail")
            .count(),
        1
    );
    assert_eq!(service.runtime_count_for_test(), 0);
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancelled_wall_commit_waits_for_started_blocking_work() {
    for _ in 0..20 {
        let (fixture, service, asset_id) = prepare_unready_fixture().await;
        let commit_started = Arc::new(tokio::sync::Notify::new());
        let commit_release = Arc::new(AtomicBool::new(false));
        service
            .install_managed_commit_started_test_gate(
                commit_started.clone(),
                commit_release.clone(),
            )
            .await;
        let visible_queued = Arc::new(tokio::sync::Notify::new());
        service
            .install_visible_derivative_queue_test_hook(visible_queued.clone())
            .await;

        service
            .set_interaction(photo_app_service::InteractionState::Idle)
            .await;
        let background_service = service.clone();
        let background_asset = asset_id.clone();
        let mut background = tokio::spawn(async move {
            background_service
                .request_derivatives(DerivativeRequest {
                    asset_ids: vec![background_asset],
                    priority: photo_app_service::DerivativePriority::NearViewport,
                    kind: DerivativeClass::WallThumbnail,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), commit_started.notified())
            .await
            .expect("wall commit should enter the blocking-work gate");

        service
            .set_interaction(photo_app_service::InteractionState::Active)
            .await;
        let visible_service = service.clone();
        let visible_asset = asset_id.clone();
        let visible = tokio::spawn(async move {
            visible_service
                .request_derivatives(DerivativeRequest::visible(vec![visible_asset]))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), visible_queued.notified())
            .await
            .expect("visible invalidation should enqueue behind started wall work");

        service.abort_derivative_attempt_for_test().await;
        tokio::task::yield_now().await;
        assert!(!background.is_finished());
        assert!(!visible.is_finished());
        commit_release.store(true, Ordering::Release);
        let background_result = tokio::time::timeout(Duration::from_secs(5), &mut background)
            .await
            .expect("supervised wall work should finish after release")
            .expect("background request task panicked");
        assert!(background_result.is_ok());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), visible)
                .await
                .expect("wall invalidation should complete after release")
                .expect("visible request task panicked")
                .is_ok()
        );
        assert_eq!(
            Catalog::open(&fixture.config.catalog_path())
                .unwrap()
                .all_derivatives()
                .unwrap()
                .into_iter()
                .filter(|record| record.kind == "wall_thumbnail")
                .count(),
            1
        );
    }
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocking_screen_commit_panic_fails_only_after_work_finishes() {
    for _ in 0..20 {
        let (fixture, service, asset_id) = prepare_wall_ready_fixture().await;
        let commit_started = Arc::new(tokio::sync::Notify::new());
        let commit_release = Arc::new(AtomicBool::new(false));
        service
            .install_managed_commit_started_test_gate(
                commit_started.clone(),
                commit_release.clone(),
            )
            .await;
        let invalidation_waiting = Arc::new(tokio::sync::Notify::new());
        service
            .install_background_invalidation_wait_test_hook(invalidation_waiting.clone())
            .await;

        service
            .set_interaction(photo_app_service::InteractionState::Idle)
            .await;
        let background_service = service.clone();
        let background_asset = asset_id.clone();
        let mut background = tokio::spawn(async move {
            background_service
                .request_derivatives(DerivativeRequest {
                    asset_ids: vec![background_asset],
                    priority: photo_app_service::DerivativePriority::NearViewport,
                    kind: DerivativeClass::ScreenPreview,
                })
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), commit_started.notified())
            .await
            .expect("screen commit should enter the blocking-work gate");

        service
            .set_interaction(photo_app_service::InteractionState::Active)
            .await;
        let visible_service = service.clone();
        let visible_asset = asset_id.clone();
        let visible = tokio::spawn(async move {
            visible_service
                .request_derivatives(DerivativeRequest::visible(vec![visible_asset]))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), invalidation_waiting.notified())
            .await
            .expect("visible invalidation should wait for started screen work");

        service
            .install_derivative_panic_in_blocking_commit_test_hook()
            .await;
        commit_release.store(true, Ordering::Release);
        let background_result = tokio::time::timeout(Duration::from_secs(5), &mut background)
            .await
            .expect("blocking panic should resolve the background waiter")
            .expect("background request task panicked");
        assert!(background_result.is_err());
        assert!(
            tokio::time::timeout(Duration::from_secs(5), visible)
                .await
                .expect("invalidation should complete after blocking panic")
                .expect("visible request task panicked")
                .is_ok()
        );
        assert_eq!(
            Catalog::open(&fixture.config.catalog_path())
                .unwrap()
                .all_derivatives()
                .unwrap()
                .into_iter()
                .filter(|record| record.kind == "screen_preview")
                .count(),
            0
        );
    }
}

#[derive(Clone)]
struct TimingWriter(Arc<Mutex<Vec<u8>>>);

impl io::Write for TimingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn derivative_timing_observer_reports_every_path_free_stage() {
    static INIT: Once = Once::new();
    let output = Arc::new(Mutex::new(Vec::new()));
    let writer = output.clone();
    INIT.call_once(|| {
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(move || TimingWriter(writer.clone()))
            .finish();
        tracing::subscriber::set_global_default(subscriber).unwrap();
    });
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open(fixture.config.clone()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut updates = service.subscribe_wall_updates();
        service.start_scan(&fixture.source).await.unwrap();
        recv_until(&mut updates, |event| {
            matches!(event, WallUpdate::MetadataSettled { .. })
        })
        .await;
        let id = service
            .query_wall(query(SortDirection::OldestFirst))
            .await
            .unwrap()
            .items[0]
            .id
            .clone();
        service
            .request_derivatives(DerivativeRequest::visible(vec![id]))
            .await
            .unwrap();
        let _ = recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 1).await;
    });
    let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    for stage in [
        "source_read_decode",
        "transform_encode",
        "managed_cache_write",
        "catalog_commit",
        "derivative_publication",
    ] {
        if let Some(line) = logs.lines().find(|line| line.contains(stage)) {
            println!("{line}");
        }
        assert!(
            logs.contains(stage),
            "missing timing stage {stage} in {logs}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unreadable_geometry_maps_to_the_stable_fallback_question_mark_state() {
    let fixture = ProgressiveFixture::new(1);
    std::fs::write(fixture.source.join("photo-000.jpg"), b"not an image").unwrap();
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = service.subscribe_wall_updates();

    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;

    let asset = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .next()
        .unwrap();
    assert_eq!((asset.width, asset.height), (4, 3));
    assert_eq!(asset.shape_state, WallShapeState::Fallback);
    assert_eq!(asset.warning.unwrap().code, "shapeFallback");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn visible_wall_work_finishes_before_screen_preview_prefetch() {
    let fixture = ProgressiveFixture::new(4);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();

    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids.clone()))
        .await
        .unwrap();

    let wall = recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 4).await;
    assert_eq!(wall.len(), 4);
    let screen = recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 4).await;
    assert_eq!(
        screen
            .iter()
            .map(|item| item.asset_id.clone())
            .collect::<std::collections::HashSet<_>>(),
        ids.iter().cloned().collect()
    );
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn screen_previews_wait_for_all_overlapping_wall_thumbnail_waves() {
    let fixture = ProgressiveFixture::new(4);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();

    let mut order_updates = service.subscribe_wall_updates();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_gate(
            ids[2].clone(),
            DerivativeClass::WallThumbnail,
            entered.clone(),
            release.clone(),
        )
        .await
        .unwrap();

    let second_service = service.clone();
    let second_ids = ids[2..].to_vec();
    let second = tokio::spawn(async move {
        second_service
            .request_derivatives(DerivativeRequest {
                asset_ids: second_ids,
                priority: photo_app_service::DerivativePriority::NearViewport,
                kind: DerivativeClass::WallThumbnail,
            })
            .await
    });
    entered.notified().await;

    let first_service = service.clone();
    let first_ids = ids[..2].to_vec();
    let first = tokio::spawn(async move {
        first_service
            .request_derivatives(DerivativeRequest::visible(first_ids))
            .await
    });

    first.await.unwrap().unwrap();
    let mut wall_ids = std::collections::HashSet::new();
    let premature_screen = tokio::time::timeout(Duration::from_millis(250), async {
        loop {
            let event = order_updates.recv().await.unwrap();
            let WallUpdate::DerivativesReady { derivatives, .. } = event else {
                continue;
            };
            for derivative in derivatives {
                match derivative.kind {
                    DerivativeClass::WallThumbnail => {
                        wall_ids.insert(derivative.asset_id);
                    }
                    DerivativeClass::ScreenPreview => return true,
                }
            }
        }
    })
    .await;
    assert!(
        premature_screen.is_err(),
        "screen previews must wait while the second wall wave remains blocked"
    );

    release.notify_waiters();
    second.await.unwrap().unwrap();
    loop {
        let event = order_updates.recv().await.unwrap();
        let WallUpdate::DerivativesReady { derivatives, .. } = event else {
            continue;
        };
        let mut screen_seen = false;
        for derivative in derivatives {
            match derivative.kind {
                DerivativeClass::WallThumbnail => {
                    wall_ids.insert(derivative.asset_id);
                    assert!(!screen_seen, "a screen preview preceded a wall thumbnail");
                }
                DerivativeClass::ScreenPreview => screen_seen = true,
            }
        }
        if screen_seen {
            assert_eq!(wall_ids.len(), 4, "all wall waves precede previews");
            break;
        }
    }
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn aborted_wall_request_releases_coordinator_for_later_request() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();

    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    let wall_entered = Arc::new(tokio::sync::Notify::new());
    let wall_release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_gate(
            asset_id.clone(),
            DerivativeClass::WallThumbnail,
            wall_entered.clone(),
            wall_release.clone(),
        )
        .await
        .unwrap();
    let first_service = service.clone();
    let first_asset = asset_id.clone();
    let first = tokio::spawn(async move {
        first_service
            .request_derivatives(DerivativeRequest::visible(vec![first_asset]))
            .await
    });
    wall_entered.notified().await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    wall_release.notify_waiters();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives, .. }
            if derivatives.iter().any(|item| item.kind == DerivativeClass::WallThumbnail))
    })
    .await;

    let screen_entered = Arc::new(tokio::sync::Notify::new());
    let screen_release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_gate(
            asset_id.clone(),
            DerivativeClass::ScreenPreview,
            screen_entered.clone(),
            screen_release.clone(),
        )
        .await
        .unwrap();
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id]))
        .await
        .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    tokio::time::timeout(Duration::from_secs(5), screen_entered.notified())
        .await
        .expect("a later request should be able to start screen previews after cancellation");
    screen_release.notify_waiters();
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn visible_request_does_not_allow_screen_prefetch_to_overtake_wall_work() {
    let fixture = ProgressiveFixture::new(5);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();
    let wall_entered = Arc::new(tokio::sync::Notify::new());
    let wall_release = Arc::new(tokio::sync::Notify::new());
    service
        .install_derivative_test_gate(
            ids[4].clone(),
            DerivativeClass::WallThumbnail,
            wall_entered.clone(),
            wall_release.clone(),
        )
        .await
        .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    tokio::time::timeout(Duration::from_secs(5), wall_entered.notified())
        .await
        .expect("full-group wall work should reach the held visible asset");
    let screen_entered = Arc::new(tokio::sync::Notify::new());
    let screen_release = Arc::new(tokio::sync::Notify::new());
    let screen_starts = Arc::new(AtomicUsize::new(0));
    service
        .install_derivative_test_class_gate_with_counter(
            DerivativeClass::ScreenPreview,
            screen_entered.clone(),
            screen_release.clone(),
            screen_starts.clone(),
        )
        .await;
    assert_eq!(
        screen_starts.load(Ordering::SeqCst),
        0,
        "screen previews must not start while a wall prerequisite is held"
    );

    let visible_started = Arc::new(tokio::sync::Notify::new());
    let visible_entry_release = Arc::new(tokio::sync::Notify::new());
    let visible_queued = Arc::new(tokio::sync::Notify::new());
    service
        .install_visible_derivative_request_test_gate(
            visible_started.clone(),
            visible_entry_release.clone(),
        )
        .await;
    service
        .install_visible_derivative_queue_test_hook(visible_queued.clone())
        .await;
    let visible_service = service.clone();
    let visible_asset = ids[4].clone();
    let visible = tokio::spawn(async move {
        visible_service
            .request_derivatives(DerivativeRequest::visible(vec![visible_asset]))
            .await
    });
    visible_started.notified().await;
    visible_entry_release.notify_one();
    visible_queued.notified().await;
    assert_eq!(
        screen_starts.load(Ordering::SeqCst),
        0,
        "a visible wall request must not be overtaken by screen work"
    );
    wall_release.notify_waiters();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives, .. }
        if derivatives.iter().any(|item| {
            item.kind == DerivativeClass::WallThumbnail && item.asset_id == ids[4]
        }))
    })
    .await;
    visible.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(5), screen_entered.notified())
        .await
        .expect("screen previews should start only after the wall release");
    screen_release.notify_waiters();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_identical_wall_request_reuses_cache_after_the_source_goes_offline() {
    let fixture = ProgressiveFixture::new(2);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids.clone()))
        .await
        .unwrap();
    let _ = recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 2).await;

    let unavailable = fixture.temp.path().join("photos-offline");
    std::fs::rename(&fixture.source, &unavailable).unwrap();
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids))
        .await
        .unwrap();
    let wall = recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 2).await;
    let screen = recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 2).await;
    assert_eq!(wall.len(), 2);
    assert_eq!(screen.len(), 2);
    std::fs::rename(unavailable, &fixture.source).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn desktop_bootstrap_offline_reopen_keeps_cached_references() {
    let fixture = ProgressiveFixture::new(4);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids))
        .await
        .unwrap();
    let _ = recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 4).await;
    drop(service);

    let unavailable = fixture.temp.path().join("photos-offline");
    std::fs::rename(&fixture.source, &unavailable).unwrap();
    let reopened =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let bootstrap = reopened.desktop_bootstrap().await.unwrap();
    let page = reopened
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(page.items.len(), 4);
    assert_eq!(page.indexed_count, 4);
    assert_eq!(page.total_count, 4);
    assert!(
        page.items
            .iter()
            .all(|asset| asset.wall_thumbnail.is_some())
    );
    reopened
        .check_saved_folders(&[bootstrap.saved_folders.active_entry_id.unwrap()])
        .await
        .unwrap();
    let confirmed = reopened
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(confirmed.items.len(), 4);
    assert!(
        confirmed
            .items
            .iter()
            .all(|asset| asset.wall_thumbnail.is_some())
    );
    std::fs::rename(unavailable, &fixture.source).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reconciliation_batches_preserve_cached_wall_and_screen_references_online_and_offline() {
    let fixture = ProgressiveFixture::new(2);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids))
        .await
        .unwrap();
    let _ = recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 2).await;
    drop(service);

    let (reader, release) = BlockingReader::new();
    let online = AppService::open_with_reader(fixture.config.clone(), Arc::new(reader)).unwrap();
    let mut updates = online.subscribe_wall_updates();
    let batch = recv_until(
        &mut updates,
        |event| matches!(event, WallUpdate::CatalogBatch { assets, .. } if !assets.is_empty()),
    )
    .await;
    assert!(matches!(batch, WallUpdate::CatalogBatch { assets, .. } if
        assets.len() == 2
        && assets.iter().all(|asset|
            asset.wall_thumbnail.is_some() && asset.screen_preview.is_some())));
    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    drop(online);

    let unavailable = fixture.temp.path().join("photos-offline-with-derivatives");
    std::fs::rename(&fixture.source, &unavailable).unwrap();
    let offline =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let page = offline
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert!(
        page.items
            .iter()
            .all(|asset| asset.wall_thumbnail.is_some() && asset.screen_preview.is_some())
    );
    std::fs::rename(unavailable, &fixture.source).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn video_only_selection_has_an_empty_photo_wall() {
    let fixture = ProgressiveFixture::video_only();
    let video_path = fixture.source.join("clip.mp4");
    let video_before = std::fs::read(&video_path).unwrap();
    let modified_before = std::fs::metadata(&video_path).unwrap().modified().unwrap();
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;

    let page = service
        .query_wall(query(SortDirection::NewestFirst))
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert!(page.next_cursor.is_none());
    assert_eq!(page.order_state, OrderState::Settled);
    assert_eq!(page.source_warnings, Vec::new());
    assert_eq!(std::fs::read(&video_path).unwrap(), video_before);
    assert_eq!(
        std::fs::metadata(&video_path).unwrap().modified().unwrap(),
        modified_before
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn derivative_requests_reject_indexed_video_ids() {
    let fixture = ProgressiveFixture::video_only();
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    let bootstrap = service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;

    let library_id = LibraryId::from_uuid(
        uuid::Uuid::parse_str(
            &bootstrap
                .active_source
                .as_ref()
                .expect("video scan should select a source")
                .id,
        )
        .unwrap(),
    );
    let video_id = AssetId::for_path(
        library_id,
        &RelativePathKey::from_relative_path(Path::new("clip.mp4")).unwrap(),
    );
    let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let video = catalog.find_asset(video_id).unwrap().unwrap();
    let group = video.folder_group_id.unwrap();
    let cache_key = photo_cache::DerivativeKey::compute(&photo_cache::DerivativeSpec {
        asset_id: video.id,
        signature: video.signature,
        media_kind: video.media_kind,
        orientation: video.orientation.unwrap_or(1),
        kind: photo_cache::DerivativeKind::WallThumbnail,
        decoder_version: photo_codec::decoder_fingerprint(video.media_kind)
            .unwrap_or_default()
            .to_owned(),
        colour_space: "srgb".to_owned(),
        target: photo_cache::DerivativeTarget::LongEdge(1024),
    });
    catalog
        .insert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id: video.id,
            folder_group_id: group,
            kind: "wall_thumbnail".to_owned(),
            cache_key: cache_key.as_str().to_owned(),
            relative_cache_path: PathBuf::from("video-placeholder.webp"),
            size_bytes: 0,
            durable: true,
            created_at: 1,
        })
        .unwrap();
    drop(catalog);

    assert!(matches!(
        service
            .request_derivatives(DerivativeRequest::visible(vec![
                video_id.as_uuid().hyphenated().to_string(),
            ]))
            .await,
        Err(photo_app_service::AppServiceError::DerivativeUnavailable)
    ));

    assert!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .all_derivatives()
            .unwrap()
            .iter()
            .any(|derivative| derivative.asset_id == video_id),
        "the cached video fixture should remain indexed"
    );
    while let Ok(event) = updates.try_recv() {
        assert!(!matches!(
            event,
            WallUpdate::DerivativesReady { derivatives, .. }
                if derivatives.iter().any(|derivative| derivative.asset_id
                    == video_id.as_uuid().hyphenated().to_string())
        ));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn returning_to_idle_resumes_remaining_screen_preview_prefetch() {
    let fixture = ProgressiveFixture::new(4);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let first = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .next()
        .unwrap()
        .id;
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(vec![first]))
        .await
        .unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives, .. } if
            derivatives.len() == 1
            && derivatives.iter().all(|item| item.kind == DerivativeClass::WallThumbnail))
    })
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                let event = updates.recv().await.unwrap();
                if matches!(event, WallUpdate::DerivativesReady { derivatives, .. } if
                    derivatives.iter().any(|item| item.kind == DerivativeClass::ScreenPreview))
                {
                    return;
                }
            }
        })
        .await
        .is_err(),
        "active interaction should pause all screen-preview prefetch"
    );

    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    let remaining = recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 3).await;
    assert_eq!(remaining.len(), 3);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn screen_preview_prefetch_waits_for_foreground_indexing_to_drain() {
    let fixture = ProgressiveFixture::new(4);
    let (reader, release) = BlockingReader::new();
    let service = fixture.service(reader);
    let mut updates = service.subscribe_wall_updates();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(
        &mut updates,
        |event| matches!(event, WallUpdate::CatalogBatch { assets, .. } if assets.len() == 4),
    )
    .await;
    let ids = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items
        .into_iter()
        .map(|asset| asset.id)
        .collect::<Vec<_>>();
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids))
        .await
        .unwrap();
    let _ = recv_derivatives_until(&mut updates, DerivativeClass::WallThumbnail, 4).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                let event = updates.recv().await.unwrap();
                if matches!(event, WallUpdate::DerivativesReady { derivatives, .. } if
                    derivatives.iter().any(|item| item.kind == DerivativeClass::ScreenPreview))
                {
                    return;
                }
            }
        })
        .await
        .is_err(),
        "screen preview work started before metadata settlement"
    );

    release.release();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let screen = recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 4).await;
    assert_eq!(screen.len(), 4);
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_photo_failures_and_videos_do_not_starve_healthy_previews() {
    let fixture = ProgressiveFixture::mixed_collection();
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;

    std::fs::remove_file(fixture.source.join("offline.jpg")).unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;

    let screens = tokio::time::timeout(
        Duration::from_secs(2),
        recv_derivatives_until(&mut updates, DerivativeClass::ScreenPreview, 2),
    )
    .await
    .expect("healthy photos should receive screen previews after terminal failures");
    assert_eq!(screens.len(), 2);

    let wall = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(
        wall.items
            .iter()
            .map(|asset| asset.display_name.as_str())
            .collect::<Vec<_>>(),
        ["a.jpg", "b.jpg", "corrupt.jpg", "offline.jpg"]
    );
    assert_eq!(wall.total_count, 4);
    assert_eq!(wall.preview_counts.wall_failed, 2);
    assert!(
        wall.items
            .iter()
            .all(|asset| asset.media_kind != WallMediaKind::Video)
    );
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_wall_failure_persists_a_nonretryable_warning() {
    let fixture = ProgressiveFixture::mixed_collection();
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;

    let warning = recv_until(&mut updates, |event| {
        matches!(
            event,
            WallUpdate::Warning {
                asset_id: Some(_),
                ..
            }
        )
    })
    .await;
    assert!(matches!(
        warning,
        WallUpdate::Warning {
            warning: WallWarningState {
                code,
                retryable: false,
            },
            ..
        } if code == "wallThumbnailUnavailable"
    ));
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn terminal_failure_retries_when_availability_changes_without_a_new_key() {
    let fixture = ProgressiveFixture::new(1);
    let service = AppService::open_with_reader(
        fixture.config.clone(),
        Arc::new(photo_indexer::DefaultMetadataReader),
    )
    .unwrap();
    let mut updates = service.subscribe_wall_updates();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let asset_id = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap()
        .items[0]
        .id
        .clone();
    let active_selection = Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .load_app_state()
        .unwrap()
        .active_selection
        .unwrap();
    Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .mark_root_offline(active_selection.library_id)
        .unwrap();

    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .expect_err("an unavailable source must terminalize the wall request");
    recv_until(&mut updates, |event| {
        matches!(
            event,
            WallUpdate::Warning {
                asset_id: Some(id),
                warning: WallWarningState {
                    retryable: false,
                    ..
                },
                ..
            } if id == &asset_id
        )
    })
    .await;

    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    let pending = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert_eq!(pending.preview_counts.wall_failed, 0);
    assert_eq!(pending.preview_counts.wall_ready, 0);
    assert!(
        pending.items[0]
            .warning
            .as_ref()
            .is_none_or(|warning| warning.retryable),
        "an available source must re-enter pending work instead of retaining an offline terminal failure"
    );
    assert!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .terminal_derivative_failures()
            .unwrap()
            .is_empty()
    );
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .expect("the same source key must retry after availability recovers");
    assert!(
        service
            .query_wall(query(SortDirection::OldestFirst))
            .await
            .unwrap()
            .items[0]
            .warning
            .is_none()
    );
    assert_eq!(
        Catalog::open(&fixture.config.catalog_path())
            .unwrap()
            .warning_count()
            .unwrap(),
        0
    );
}

#[test]
fn wall_dtos_never_serialize_native_paths() {
    let page = WallPage {
        items: vec![WallAsset {
            id: "asset-id".to_owned(),
            display_name: "photo.jpg".to_owned(),
            media_kind: WallMediaKind::Jpeg,
            provisional_order: 7,
            captured_at_utc: Some("2025-01-02T03:04:05Z".to_owned()),
            date_state: OrderState::Settled,
            width: 4_032,
            height: 3_024,
            representative_rgb: Some(0x123456),
            shape_state: WallShapeState::Ready,
            availability: SourceAvailability::Available,
            warning: Some(WallWarningState {
                code: "derivativeUnavailable".to_owned(),
                retryable: true,
            }),
            wall_thumbnail: Some(DerivativeReference {
                asset_id: "asset-id".to_owned(),
                kind: DerivativeClass::WallThumbnail,
                key: "opaque-cache-key".to_owned(),
            }),
            screen_preview: None,
            rating: None,
        }],
        next_cursor: Some("opaque-cursor".to_owned()),
        order_state: OrderState::Settled,
        source_warnings: Vec::new(),
        total_count: 1,
        indexed_count: 1,
        preview_counts: photo_app_service::WallPreviewCounts::default(),
    };
    let json = serde_json::to_string(&page).unwrap();
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("\\\\server\\share"));
    assert!(!json.contains("relativeCachePath"));
    assert!(json.contains("\"displayName\":\"photo.jpg\""));
    assert!(json.contains("\"representativeRgb\":1193046"));
    assert!(json.contains("\"screenPreview\":null"));
}
