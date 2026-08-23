mod eviction;
mod key;
mod writer;

pub use eviction::{EvictionPlan, EvictionPlanner, ProtectedGroups};
pub use key::{DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget};
pub use writer::{CacheReconcileReport, CacheWrite, CacheWriter};

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error("cache path would escape the configured root")]
    PathEscape,
    #[error("cache write failed: {0}")]
    Write(#[source] std::io::Error),
    #[error("cache filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("catalog operation failed: {0}")]
    Catalog(#[from] photo_catalog::CatalogError),
    #[error("cache size cannot be represented on this platform")]
    SizeOutOfRange,
}
