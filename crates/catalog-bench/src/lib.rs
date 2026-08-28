use std::path::{Path, PathBuf};
use std::time::Instant;

use photo_cache::{EvictionPlanner, ProtectedGroups};
use photo_catalog::{
    AssetMetadataUpdate, AssetShapeUpdate, Catalog, CatalogIndexRecord, NewAsset, NewDerivative,
    NewFolderGroup, NewLibrary, ShapeStatus, TerminalDerivativeFailure, WallOrder,
};
use photo_domain::{
    AssetId, DerivativeId, FileSignature, FolderGroupId, LibraryId, LibraryKind, MediaKind,
    NativePathKey, RelativePathKey,
};
use serde::Serialize;

const BENCHMARK_LIBRARY_ID: u128 = 0x6cdb_80d2_6763_4e72_845c_753f_65bf_a001;
const GROUP_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x7814_b18f_61e0_4a3a_8aa9_55b9_7367_1001);
const DERIVATIVE_NAMESPACE: uuid::Uuid =
    uuid::Uuid::from_u128(0x7814_b18f_61e0_4a3a_8aa9_55b9_7367_1002);
const WALL_GROUP_PATH: &str = "benchmark-wall";
const WALL_PAGE_SIZE: u32 = 100;
const COORDINATOR_PAGE_SIZE: u32 = 250;
const VIDEO_PERIOD: u64 = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BenchmarkConfig {
    pub assets: u64,
    pub batch_size: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BenchmarkReport {
    pub assets: u64,
    pub sqlite_version: String,
    pub database_bytes: u64,
    pub insert_ms: f64,
    pub first_page_ms: f64,
    pub first_page_rows: usize,
    pub second_page_ms: f64,
    pub second_page_rows: usize,
    pub coordinator_page_ms: f64,
    pub coordinator_page_rows: usize,
    pub terminal_lookup_ms: f64,
    pub unavailable_count_ms: f64,
    pub eviction_plan_ms: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum BenchmarkError {
    #[error("benchmark batch size must be greater than zero")]
    EmptyBatch,
    #[error("catalog benchmark failed: {0}")]
    Catalog(#[from] photo_catalog::CatalogError),
    #[error("cache benchmark failed: {0}")]
    Cache(#[from] photo_cache::CacheError),
    #[error("benchmark filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("benchmark generated {actual} rows instead of {expected}")]
    IncorrectRowCount { expected: u64, actual: u64 },
    #[error("benchmark wall page did not provide a continuation cursor")]
    MissingWallCursor,
    #[error("benchmark terminal failure lookup did not find the current key")]
    MissingTerminalFailure,
    #[error("benchmark photo page contained a video result")]
    UnexpectedVideo,
    #[error("benchmark value is too large for this platform")]
    ValueOutOfRange,
}

pub fn run_benchmark(config: BenchmarkConfig) -> Result<BenchmarkReport, BenchmarkError> {
    if config.batch_size == 0 {
        return Err(BenchmarkError::EmptyBatch);
    }

    let temp = tempfile::tempdir()?;
    let database_path = temp.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&database_path)?;
    let sqlite_version = catalog.sqlite_version()?;
    let library = benchmark_library(&temp.path().join("source-placeholder"));
    catalog.add_library(&library)?;
    let wall_group = insert_wall_group(&mut catalog, library.id)?;

    let insert_started = Instant::now();
    let mut offset = 0_u64;
    let mut video_count = 0_u64;
    while offset < config.assets {
        let remaining = config.assets - offset;
        let batch_assets = remaining
            .min(u64::try_from(config.batch_size).map_err(|_| BenchmarkError::ValueOutOfRange)?);
        let mut records = Vec::with_capacity(
            usize::try_from(batch_assets)
                .map_err(|_| BenchmarkError::ValueOutOfRange)?
                .saturating_mul(3),
        );
        for index in offset..offset + batch_assets {
            let asset = benchmark_asset(library.id, wall_group, index)?;
            let asset_id = asset.id;
            if asset.media_kind == MediaKind::Video {
                video_count += 1;
            }
            records.push(CatalogIndexRecord::Discovered(asset));
            records.push(CatalogIndexRecord::Shaped(AssetShapeUpdate {
                asset_id,
                width: 4_000,
                height: 3_000,
                orientation: Some(1),
                representative_rgb: Some(0x335577),
                shape_status: ShapeStatus::Ready,
            }));
            records.push(CatalogIndexRecord::Metadata(AssetMetadataUpdate {
                asset_id,
                captured_at_utc: None,
                rating: Some(rating(index)),
                keywords: Vec::new(),
                provenance: Vec::new(),
            }));
        }
        catalog.apply_index_batch(&records)?;
        offset += batch_assets;
    }
    if video_count != config.assets.div_ceil(VIDEO_PERIOD) {
        return Err(BenchmarkError::IncorrectRowCount {
            expected: config.assets.div_ceil(VIDEO_PERIOD),
            actual: video_count,
        });
    }
    insert_cache_groups(&mut catalog, library.id, config)?;
    let insert_ms = elapsed_ms(insert_started);

    let actual = catalog.asset_count(library.id)?;
    if actual != config.assets {
        return Err(BenchmarkError::IncorrectRowCount {
            expected: config.assets,
            actual,
        });
    }

    let page_started = Instant::now();
    let first_page = catalog.wall_page(wall_group, WallOrder::Provisional, None, WALL_PAGE_SIZE)?;
    let first_page_ms = elapsed_ms(page_started);
    ensure_photo_page(&first_page.items, WALL_PAGE_SIZE)?;
    let second_cursor = first_page
        .next
        .clone()
        .ok_or(BenchmarkError::MissingWallCursor)?;
    let second_started = Instant::now();
    let second_page = catalog.wall_page(
        wall_group,
        WallOrder::Provisional,
        Some(second_cursor),
        WALL_PAGE_SIZE,
    )?;
    let second_page_ms = elapsed_ms(second_started);
    ensure_photo_page(&second_page.items, WALL_PAGE_SIZE)?;
    let coordinator_started = Instant::now();
    let coordinator_page = catalog.photo_asset_ids_page(
        wall_group,
        WallOrder::Provisional,
        None,
        COORDINATOR_PAGE_SIZE,
    )?;
    let coordinator_page_ms = elapsed_ms(coordinator_started);
    if coordinator_page.items.len() != COORDINATOR_PAGE_SIZE as usize {
        return Err(BenchmarkError::IncorrectRowCount {
            expected: u64::from(COORDINATOR_PAGE_SIZE),
            actual: coordinator_page.items.len() as u64,
        });
    }
    for asset_id in &coordinator_page.items {
        let asset = catalog
            .find_asset(*asset_id)?
            .ok_or(BenchmarkError::IncorrectRowCount {
                expected: 1,
                actual: 0,
            })?;
        if asset.media_kind == MediaKind::Video {
            return Err(BenchmarkError::UnexpectedVideo);
        }
    }

    let retained = catalog.mark_root_offline(library.id)?;
    if retained != config.assets {
        return Err(BenchmarkError::IncorrectRowCount {
            expected: config.assets,
            actual: retained,
        });
    }
    let unavailable_started = Instant::now();
    let unavailable = catalog.unavailable_asset_count(library.id)?;
    let unavailable_count_ms = elapsed_ms(unavailable_started);
    if unavailable != config.assets {
        return Err(BenchmarkError::IncorrectRowCount {
            expected: config.assets,
            actual: unavailable,
        });
    }

    let terminal_asset = catalog
        .find_asset(first_page.items[0].id)?
        .ok_or(BenchmarkError::MissingTerminalFailure)?;
    let terminal_key = format!("screen-preview-{}", terminal_asset.id.as_uuid().simple());
    catalog.record_terminal_derivative_failure(&TerminalDerivativeFailure {
        asset_id: terminal_asset.id,
        kind: "screen_preview".to_owned(),
        cache_key: terminal_key.clone(),
        availability: terminal_asset.availability,
        failure_code: "benchmark_terminal".to_owned(),
        occurred_at: 1,
    })?;
    let terminal_started = Instant::now();
    let terminal_failure = catalog.find_terminal_derivative_failure(
        terminal_asset.id,
        "screen_preview",
        &terminal_key,
        terminal_asset.availability,
    )?;
    let terminal_lookup_ms = elapsed_ms(terminal_started);
    if terminal_failure.is_none() {
        return Err(BenchmarkError::MissingTerminalFailure);
    }

    let eviction_started = Instant::now();
    let _plan = EvictionPlanner::plan(
        &catalog,
        10_u64.saturating_mul(1024).saturating_mul(1024),
        &ProtectedGroups::default(),
    )?;
    let eviction_plan_ms = elapsed_ms(eviction_started);

    drop(catalog);
    let database_bytes = std::fs::metadata(&database_path)?.len();
    Ok(BenchmarkReport {
        assets: config.assets,
        sqlite_version: format!(
            "{}.{}.{}",
            sqlite_version.major, sqlite_version.minor, sqlite_version.patch
        ),
        database_bytes,
        insert_ms,
        first_page_ms,
        first_page_rows: first_page.items.len(),
        second_page_ms,
        second_page_rows: second_page.items.len(),
        coordinator_page_ms,
        coordinator_page_rows: coordinator_page.items.len(),
        terminal_lookup_ms,
        unavailable_count_ms,
        eviction_plan_ms,
    })
}

fn ensure_photo_page(
    rows: &[photo_catalog::WallCatalogRecord],
    expected: u32,
) -> Result<(), BenchmarkError> {
    if rows.len() != expected as usize {
        return Err(BenchmarkError::IncorrectRowCount {
            expected: u64::from(expected),
            actual: rows.len() as u64,
        });
    }
    if rows.iter().any(|row| row.media_kind == MediaKind::Video) {
        return Err(BenchmarkError::UnexpectedVideo);
    }
    Ok(())
}

fn benchmark_library(root: &Path) -> NewLibrary {
    NewLibrary {
        id: LibraryId::from_uuid(uuid::Uuid::from_u128(BENCHMARK_LIBRARY_ID)),
        kind: LibraryKind::Configured,
        display_name: "Benchmark Library".to_owned(),
        canonical_root_key: NativePathKey::from_path(root),
        display_path: "benchmark-library".to_owned(),
    }
}

fn benchmark_asset(
    library: LibraryId,
    wall_group: FolderGroupId,
    index: u64,
) -> Result<NewAsset, BenchmarkError> {
    let path = benchmark_path(index);
    let relative_path = RelativePathKey::from_relative_path(Path::new(&path))
        .map_err(|_| BenchmarkError::ValueOutOfRange)?;
    Ok(NewAsset {
        id: AssetId::for_path(library, &relative_path),
        library_id: library,
        relative_path,
        display_path: path,
        media_kind: media_kind(index),
        signature: FileSignature {
            size_bytes: 1_000_000 + (index % 25_000_000),
            modified_unix_ns: i128::from(index).saturating_mul(1_000_000_000),
            sidecar_modified_unix_ns: None,
        },
        folder_group_id: Some(wall_group),
    })
}

fn benchmark_path(index: u64) -> String {
    let year = 2000 + (index % 27);
    let collection = index % 10_000;
    let rating = rating(index);
    let extension = match media_kind(index) {
        MediaKind::Raw => "cr2",
        MediaKind::Video => "mp4",
        _ => "jpg",
    };
    format!("{year}/Collection-{collection:04}/Processed/{rating} Stars/IMG-{index:07}.{extension}")
}

fn rating(index: u64) -> u8 {
    u8::try_from((index % 5) + 1).expect("rating is always between one and five")
}

fn media_kind(index: u64) -> MediaKind {
    if index.is_multiple_of(VIDEO_PERIOD) {
        MediaKind::Video
    } else if index % 20 == 1 {
        MediaKind::Raw
    } else {
        MediaKind::Jpeg
    }
}

fn insert_wall_group(
    catalog: &mut Catalog,
    library: LibraryId,
) -> Result<FolderGroupId, BenchmarkError> {
    let group_uuid = uuid::Uuid::new_v5(&GROUP_NAMESPACE, WALL_GROUP_PATH.as_bytes());
    let group_id = FolderGroupId::from_uuid(group_uuid);
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: group_id,
            library_id: library,
            relative_path: RelativePathKey::from_relative_path(Path::new(WALL_GROUP_PATH))
                .map_err(|_| BenchmarkError::ValueOutOfRange)?,
            display_path: WALL_GROUP_PATH.to_owned(),
            last_viewed_at: None,
        })
        .map_err(BenchmarkError::from)
}

fn insert_cache_groups(
    catalog: &mut Catalog,
    library: LibraryId,
    config: BenchmarkConfig,
) -> Result<(), BenchmarkError> {
    let batch_size =
        u64::try_from(config.batch_size).map_err(|_| BenchmarkError::ValueOutOfRange)?;
    let group_count = config.assets.div_ceil(batch_size);
    for group_index in 0..group_count {
        let path = format!("cache-group-{group_index:07}");
        let group_uuid = uuid::Uuid::new_v5(&GROUP_NAMESPACE, path.as_bytes());
        let group_id = FolderGroupId::from_uuid(group_uuid);
        let group_id = catalog.upsert_folder_group(&NewFolderGroup {
            id: group_id,
            library_id: library,
            relative_path: RelativePathKey::from_relative_path(Path::new(&path))
                .map_err(|_| BenchmarkError::ValueOutOfRange)?,
            display_path: path.clone(),
            last_viewed_at: (group_index > 0).then_some(
                i64::try_from(group_index).map_err(|_| BenchmarkError::ValueOutOfRange)?,
            ),
        })?;
        let asset_index = group_index.saturating_mul(batch_size);
        let asset_path = benchmark_path(asset_index);
        let asset_key = RelativePathKey::from_relative_path(Path::new(&asset_path))
            .map_err(|_| BenchmarkError::ValueOutOfRange)?;
        let derivative_uuid = uuid::Uuid::new_v5(&DERIVATIVE_NAMESPACE, path.as_bytes());
        catalog.insert_derivative(&NewDerivative {
            id: DerivativeId::from_uuid(derivative_uuid),
            asset_id: AssetId::for_path(library, &asset_key),
            folder_group_id: group_id,
            kind: "screen_preview".to_owned(),
            cache_key: derivative_uuid.simple().to_string(),
            relative_cache_path: PathBuf::from(format!("aa/bb/{derivative_uuid}.preview")),
            size_bytes: 1024 * 1024,
            durable: false,
            created_at: i64::try_from(group_index).map_err(|_| BenchmarkError::ValueOutOfRange)?,
        })?;
    }
    Ok(())
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}
