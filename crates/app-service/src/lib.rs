mod config;
pub(crate) mod derivative_coordinator;
mod derivatives;
mod dto;
mod folder_access;
mod gallery;
mod hosted_runtime;
mod original_copy;
mod picks;
mod saved_folders;
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
    WallPreviewCounts, WallQueryRequest, WallShapeState, WallUpdate, WallWarningState,
};
pub use dto::{PickItem, PickListSnapshot, PickReference};
pub use folder_access::{
    AccessReply, FolderAccessCoordinator, FolderAccessKey, FolderAccessTarget, FolderProbe,
    FolderProbeOutcome, ValidatedFolder,
};
#[cfg(feature = "server-internal-prevalidated-source")]
#[doc(hidden)]
pub use gallery::PrevalidatedHostedSource;
pub use gallery::{
    FolderBreadcrumb, GalleryEngine, GallerySelection, ManagedDerivative, SelectionSummary,
};
pub use hosted_runtime::SelectionEventSubscription;
pub use hosted_runtime::SequencedWallUpdate;
pub use original_copy::{CopyItemResult, CopyItemStatus, OriginalCopyBatch, OriginalCopyResult};
pub use photo_domain::GalleryScope;
pub use photo_indexer::MetadataReader;
pub use service::{AppService, AppServiceError};

pub use dto::{FolderAccess, FolderAccessState, SavedFolder, SavedFolderSnapshot};
