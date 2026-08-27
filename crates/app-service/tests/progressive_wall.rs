use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
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
use photo_catalog::Catalog;
use photo_metadata::{MetadataBundle, MetadataCandidate, MetadataReadWarning, MetadataSource};

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

fn query(direction: SortDirection) -> WallQueryRequest {
    WallQueryRequest {
        cursor: None,
        limit: 50,
        direction,
    }
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unavailable_reopen_retains_cached_wall_and_emits_source_unavailable() {
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
    let cached = reopened
        .query_wall(query(SortDirection::NewestFirst))
        .await
        .unwrap();
    assert_eq!(cached.items.len(), 12);
    let mut updates = reopened.subscribe_wall_updates();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::SourceUnavailable { .. })
    })
    .await;
    let unavailable_page = reopened
        .query_wall(query(SortDirection::NewestFirst))
        .await
        .unwrap();
    assert_eq!(unavailable_page.items.len(), 12);
    assert!(unavailable_page.items.iter().all(|asset| {
        asset.availability == SourceAvailability::RootOffline
            && asset
                .warning
                .as_ref()
                .is_some_and(|warning| warning.retryable)
    }));
    std::fs::rename(unavailable, &fixture.source).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unavailable_selected_child_marks_only_its_group_offline() {
    let fixture = ProgressiveFixture::new(4);
    let child = fixture.source.join("child");
    std::fs::create_dir(&child).unwrap();
    std::fs::copy(
        fixture.source.join("photo-000.jpg"),
        child.join("child.jpg"),
    )
    .unwrap();
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
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
    drop(service);

    let unavailable = fixture.source.join("child-offline");
    std::fs::rename(&child, &unavailable).unwrap();
    let reopened =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
            .unwrap();
    let mut updates = reopened.subscribe_wall_updates();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::SourceUnavailable { .. })
    })
    .await;

    let catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
    let selection = catalog.load_app_state().unwrap().active_selection.unwrap();
    let child_group = catalog
        .folder_group_for_path(selection.library_id, &selection.relative_folder)
        .unwrap()
        .unwrap();
    let root_group = catalog
        .folder_group_for_path(
            selection.library_id,
            &photo_domain::RelativePathKey::from_relative_path(Path::new("")).unwrap(),
        )
        .unwrap()
        .unwrap();
    let assets = catalog.assets(selection.library_id).unwrap();
    assert!(
        assets
            .iter()
            .any(|asset| asset.folder_group_id == Some(child_group))
    );
    assert!(
        assets
            .iter()
            .filter(|asset| asset.folder_group_id == Some(child_group))
            .all(|asset| asset.availability == photo_domain::Availability::RootOffline)
    );
    assert!(
        assets
            .iter()
            .filter(|asset| asset.folder_group_id == Some(root_group))
            .all(|asset| asset.availability == photo_domain::Availability::Available)
    );
    assert_eq!(
        catalog
            .find_library(selection.library_id)
            .unwrap()
            .unwrap()
            .availability,
        photo_domain::Availability::Available
    );
    std::fs::rename(unavailable, child).unwrap();
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
    let before = service.preview_gate_generation_test();
    service
        .request_derivatives(DerivativeRequest::visible_screen_preview(vec![asset_id]))
        .await
        .unwrap();
    let after = service.preview_gate_generation_test();
    assert_eq!(after, before);
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
async fn derivative_failure_records_and_publishes_a_retryable_asset_warning() {
    let fixture = ProgressiveFixture::new(1);
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
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
            warning: WallWarningState { retryable: true, .. },
            ..
        } if id == asset_id
    ));
    let page = service
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert!(
        page.items[0]
            .warning
            .as_ref()
            .is_some_and(|warning| warning.retryable && warning.code == "derivativeUnavailable")
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

    std::fs::write(fixture.source.join("photo-000.jpg"), original).unwrap();
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
async fn repeated_derivative_failures_return_errors_until_the_asset_recovers() {
    let fixture = ProgressiveFixture::new(1);
    let service =
        AppService::open_with_reader(fixture.config.clone(), Arc::new(CountingReader::default()))
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

    std::fs::write(fixture.source.join("photo-000.jpg"), original).unwrap();
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    let ready = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives, .. } if derivatives.iter().any(|item| item.asset_id == asset_id && item.kind == DerivativeClass::WallThumbnail))
    })
    .await;
    assert!(matches!(ready, WallUpdate::DerivativesReady { .. }));
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
async fn full_group_prefetch_completes_all_wall_pages_before_any_screen_preview() {
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
        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
        .await
        .unwrap();
    service
        .set_interaction(photo_app_service::InteractionState::Active)
        .await;
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
async fn aborted_wall_request_releases_preview_gate_for_later_request() {
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
    let gate_generation_before_scan = service.preview_gate_generation_test();
    service.start_scan(&fixture.source).await.unwrap();
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::MetadataSettled { .. })
    })
    .await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if service.preview_gate_generation_test() >= gate_generation_before_scan + 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("scan completion should schedule the full-group preview pass");
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

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn blocked_preview_gate_parks_until_interaction_transition() {
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
    service.reset_preview_gate_test_checks();
    service
        .request_derivatives(DerivativeRequest::visible(vec![asset_id]))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(180)).await;
    let checks_while_active = service.preview_gate_test_checks();
    assert_eq!(
        checks_while_active, 1,
        "a closed gate should park instead of polling on a retry timer"
    );

    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives, .. }
            if derivatives.iter().any(|item| item.kind == DerivativeClass::ScreenPreview))
    })
    .await;
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
async fn offline_reopen_keeps_cached_references() {
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
    let page = reopened
        .query_wall(query(SortDirection::OldestFirst))
        .await
        .unwrap();
    assert!(
        page.items
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
    };
    let json = serde_json::to_string(&page).unwrap();
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("\\\\server\\share"));
    assert!(!json.contains("relativeCachePath"));
    assert!(json.contains("\"displayName\":\"photo.jpg\""));
    assert!(json.contains("\"representativeRgb\":1193046"));
    assert!(json.contains("\"screenPreview\":null"));
}
