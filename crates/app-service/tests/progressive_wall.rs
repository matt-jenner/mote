use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use chrono::{FixedOffset, TimeZone};
use image::{ImageBuffer, Rgb};
use photo_app_service::{
    AppConfig, AppService, DerivativeClass, DerivativeReference, MetadataReader, OrderState,
    SortDirection, SourceAvailability, WallAsset, WallMediaKind, WallPage, WallQueryRequest,
    WallShapeState, WallUpdate, WallWarningState,
};
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
    let mut last = None;
    loop {
        match tokio::time::timeout(Duration::from_secs(2), receiver.recv()).await {
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

fn query(direction: SortDirection) -> WallQueryRequest {
    WallQueryRequest {
        cursor: None,
        limit: 50,
        direction,
    }
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
    let ready = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { .. })
    })
    .await;
    match ready {
        WallUpdate::DerivativesReady { derivatives } => {
            assert_eq!(derivatives.len(), 8);
            assert_eq!(
                derivatives
                    .iter()
                    .map(|d| d.asset_id.clone())
                    .collect::<Vec<_>>(),
                ids
            );
        }
        _ => unreachable!(),
    }
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids))
        .await
        .unwrap();
    let second = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { .. })
    })
    .await;
    assert!(
        matches!(second, WallUpdate::DerivativesReady { ref derivatives } if derivatives.len() == 8)
    );
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
    std::fs::write(
        fixture.source.join("photo-000.jpg"),
        b"damaged after indexing",
    )
    .unwrap();

    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(vec![
            asset_id.clone(),
        ]))
        .await
        .unwrap();

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
            .is_some_and(|warning| warning.retryable)
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

    let wall = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { .. })
    })
    .await;
    assert!(
        matches!(wall, WallUpdate::DerivativesReady { derivatives } if
        derivatives.len() == 4
        && derivatives.iter().all(|item| item.kind == DerivativeClass::WallThumbnail))
    );
    let screen = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            !derivatives.is_empty()
            && derivatives.iter().all(|item| item.kind == DerivativeClass::ScreenPreview))
    })
    .await;
    assert!(
        matches!(screen, WallUpdate::DerivativesReady { derivatives } if
        derivatives.iter().map(|item| item.asset_id.clone()).collect::<Vec<_>>() == ids)
    );
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
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            derivatives.iter().all(|item| item.kind == DerivativeClass::ScreenPreview))
    })
    .await;

    let unavailable = fixture.temp.path().join("photos-offline");
    std::fs::rename(&fixture.source, &unavailable).unwrap();
    service
        .request_derivatives(photo_app_service::DerivativeRequest::visible(ids))
        .await
        .unwrap();
    let wall = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            !derivatives.is_empty()
            && derivatives.iter().all(|item| item.kind == DerivativeClass::WallThumbnail))
    })
    .await;
    let screen = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            !derivatives.is_empty()
            && derivatives.iter().all(|item| item.kind == DerivativeClass::ScreenPreview))
    })
    .await;
    assert!(matches!(wall, WallUpdate::DerivativesReady { derivatives } if derivatives.len() == 2));
    assert!(
        matches!(screen, WallUpdate::DerivativesReady { derivatives } if derivatives.len() == 2)
    );
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
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            derivatives.len() == 4
            && derivatives.iter().all(|item| item.kind == DerivativeClass::WallThumbnail))
    })
    .await;
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
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            derivatives.len() == 1
            && derivatives.iter().all(|item| item.kind == DerivativeClass::ScreenPreview))
    })
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                let event = updates.recv().await.unwrap();
                if matches!(event, WallUpdate::DerivativesReady { derivatives } if
                    derivatives.iter().any(|item| item.kind == DerivativeClass::ScreenPreview))
                {
                    return;
                }
            }
        })
        .await
        .is_err(),
        "active interaction should pause the remaining-group prefetch"
    );

    service
        .set_interaction(photo_app_service::InteractionState::Idle)
        .await;
    let remaining = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            derivatives.iter().all(|item| item.kind == DerivativeClass::ScreenPreview))
    })
    .await;
    assert!(
        matches!(remaining, WallUpdate::DerivativesReady { derivatives } if derivatives.len() == 3)
    );
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
    recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            derivatives.iter().all(|item| item.kind == DerivativeClass::WallThumbnail))
    })
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(150), async {
            loop {
                let event = updates.recv().await.unwrap();
                if matches!(event, WallUpdate::DerivativesReady { derivatives } if
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
    let screen = recv_until(&mut updates, |event| {
        matches!(event, WallUpdate::DerivativesReady { derivatives } if
            derivatives.iter().all(|item| item.kind == DerivativeClass::ScreenPreview))
    })
    .await;
    assert!(
        matches!(screen, WallUpdate::DerivativesReady { derivatives } if derivatives.len() == 4)
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
        }],
        next_cursor: Some("opaque-cursor".to_owned()),
        order_state: OrderState::Settled,
    };
    let json = serde_json::to_string(&page).unwrap();
    assert!(!json.contains("/Users/"));
    assert!(!json.contains("\\\\server\\share"));
    assert!(!json.contains("relativeCachePath"));
    assert!(json.contains("\"displayName\":\"photo.jpg\""));
    assert!(json.contains("\"representativeRgb\":1193046"));
    assert!(json.contains("\"screenPreview\":null"));
}
