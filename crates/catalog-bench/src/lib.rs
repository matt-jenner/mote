use std::path::{Path, PathBuf};
use std::time::Instant;

use photo_cache::{EvictionPlanner, ProtectedGroups};
use photo_catalog::{
    AssetMetadataUpdate, Catalog, CatalogIndexRecord, NewAsset, NewDerivative, NewFolderGroup,
    NewLibrary,
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

    let insert_started = Instant::now();
    let mut offset = 0_u64;
    while offset < config.assets {
        let remaining = config.assets - offset;
        let batch_assets = remaining
            .min(u64::try_from(config.batch_size).map_err(|_| BenchmarkError::ValueOutOfRange)?);
        let mut records = Vec::with_capacity(
            usize::try_from(batch_assets)
                .map_err(|_| BenchmarkError::ValueOutOfRange)?
                .saturating_mul(2),
        );
        for index in offset..offset + batch_assets {
            let asset = benchmark_asset(library.id, index)?;
            let asset_id = asset.id;
            records.push(CatalogIndexRecord::Discovered(asset));
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
    let first_page = catalog.list_assets_page(library.id, None, 100)?;
    let first_page_ms = elapsed_ms(page_started);
    let expected_page_rows =
        usize::try_from(config.assets.min(100)).map_err(|_| BenchmarkError::ValueOutOfRange)?;
    if first_page.len() != expected_page_rows {
        return Err(BenchmarkError::IncorrectRowCount {
            expected: expected_page_rows as u64,
            actual: first_page.len() as u64,
        });
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
        first_page_rows: first_page.len(),
        unavailable_count_ms,
        eviction_plan_ms,
    })
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

fn benchmark_asset(library: LibraryId, index: u64) -> Result<NewAsset, BenchmarkError> {
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
        folder_group_id: None,
    })
}

fn benchmark_path(index: u64) -> String {
    let year = 2000 + (index % 27);
    let collection = index % 10_000;
    let rating = rating(index);
    format!("{year}/Collection-{collection:04}/Processed/{rating} Stars/IMG-{index:07}.jpg")
}

fn rating(index: u64) -> u8 {
    u8::try_from((index % 5) + 1).expect("rating is always between one and five")
}

fn media_kind(index: u64) -> MediaKind {
    match index % 20 {
        0 => MediaKind::Raw,
        1 => MediaKind::Video,
        _ => MediaKind::Jpeg,
    }
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
