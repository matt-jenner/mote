mod asset_repo;
mod cache_repo;
mod connection;
mod derivative_failure_repo;
mod generation_repo;
mod health_repo;
mod index_repo;
mod library_repo;
mod migrate;
mod policy_repo;
mod selection_repo;
mod settings_repo;
mod wall_repo;

use std::num::ParseIntError;
use std::path::Path;

pub use asset_repo::{AssetRecord, NewAsset};
pub use cache_repo::{CacheEvictionGroup, DerivativeRecord, NewDerivative, NewFolderGroup};
pub use derivative_failure_repo::TerminalDerivativeFailure;
pub use generation_repo::GenerationCompletion;
pub use health_repo::CatalogHealthSnapshot;
pub use index_repo::{
    AssetColourUpdate, AssetMetadataUpdate, AssetShapeUpdate, CatalogIndexRecord, CatalogKeyword,
    CatalogProvenance, CatalogWarningRecord, CatalogWarningSummary,
};
pub use library_repo::{LibraryRootRecord, NewLibrary};
use rusqlite::Connection;
pub use selection_repo::FolderGroupRecord;
pub use settings_repo::{AppStateRecord, StoredSourceSelection};
use thiserror::Error;
pub use wall_repo::{
    PhotoAssetIdPage, ShapeStatus, WallCatalogPage, WallCatalogRecord, WallCursorKey, WallOrder,
};

pub struct Catalog {
    pub(crate) connection: Connection,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SqliteVersion {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl SqliteVersion {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("catalog I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("SQLite operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("catalog JSON is invalid: {0}")]
    Json(#[from] serde_json::Error),
    #[error("catalog migration failed: {0}")]
    MigrationFailed(String),
    #[error("SQLite {found:?} is older than the required {minimum:?}")]
    UnsafeSqliteVersion {
        found: SqliteVersion,
        minimum: SqliteVersion,
    },
    #[error("invalid SQLite version string: {0}")]
    InvalidSqliteVersion(String),
    #[error("catalog contains invalid data: {0}")]
    InvalidData(String),
    #[error("numeric value cannot be stored in SQLite")]
    ValueOutOfRange,
    #[error("wall cursor variant does not match the requested order")]
    WallCursorOrderMismatch,
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self, CatalogError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = migrate::migrate_with(path, migrate::MIGRATIONS)?;
        Ok(Self { connection })
    }

    pub fn open_in_memory() -> Result<Self, CatalogError> {
        let mut connection = Connection::open_in_memory()?;
        connection::ensure_safe_sqlite(&connection)?;
        connection::configure(&connection, false)?;
        migrate::apply_migrations(&mut connection, migrate::MIGRATIONS)?;
        Ok(Self { connection })
    }

    pub fn sqlite_version(&self) -> Result<SqliteVersion, CatalogError> {
        connection::sqlite_version(&self.connection)
    }

    pub fn warning_count(&self) -> Result<u64, CatalogError> {
        self.connection
            .query_row("SELECT COUNT(*) FROM warnings", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|count| count as u64)
            .map_err(CatalogError::from)
    }

    pub fn journal_mode(&self) -> Result<String, CatalogError> {
        self.connection
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .map_err(CatalogError::from)
    }
}

pub(crate) fn parse_i128(value: String) -> Result<i128, rusqlite::Error> {
    value.parse().map_err(|error: ParseIntError| {
        rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
    })
}
