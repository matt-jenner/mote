mod budget;
mod eviction;
mod image_derivative;
mod key;
mod writer;

pub use budget::CacheBudget;
pub use eviction::{ActiveWriteGuard, EvictionPlan, EvictionPlanner, ProtectedGroups};
pub use image_derivative::{GeneratedDerivative, ImageDerivativeError, ImageDerivativeGenerator};
pub use key::{DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget};
pub use photo_metadata::RepresentativeRgb;
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
    #[error("cache protection state is unavailable")]
    ProtectionUnavailable,
    #[error("an eviction group became protected before deletion")]
    GroupBecameProtected,
}
