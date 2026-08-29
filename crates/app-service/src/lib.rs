mod config;
pub(crate) mod derivative_coordinator;
mod derivatives;
mod dto;
mod gallery;
mod hosted_runtime;
mod scan;
mod service;
mod wall;

pub use config::AppConfig;
#[cfg(debug_assertions)]
pub use derivative_coordinator::{CollectionPhase, CoordinatorTestSnapshot};
pub use dto::{BootstrapState, SettingsState, SourceAvailability, SourceSummary};
pub use dto::{
    DerivativeClass, DerivativePriority, DerivativeReference, DerivativeRequest, InteractionState,
    OrderState, ScanProgressDto, SortDirection, WallAsset, WallMediaKind, WallPage,
    WallQueryRequest, WallShapeState, WallUpdate, WallWarningState,
};
pub use gallery::{FolderBreadcrumb, GalleryEngine, GallerySelection, SelectionSummary};
pub use hosted_runtime::SequencedWallUpdate;
pub use photo_domain::GalleryScope;
pub use photo_indexer::MetadataReader;
pub use service::{AppService, AppServiceError};
