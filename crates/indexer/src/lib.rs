mod catalog_writer;
mod default_reader;
mod discover;
mod events;
mod reconcile;
mod scanner;
mod scheduler;
mod watch;

use std::path::PathBuf;

pub use catalog_writer::CatalogWriter;
pub use default_reader::DefaultMetadataReader;
pub use discover::find_sidecar;
pub use events::{IndexEvent, ScanProgress, ScanStage, ScanSummary};
pub use reconcile::{
    RealReconcileSource, ReconcileError, ReconcileOutcome, ReconcileSource, Reconciler,
};
pub use scanner::{Indexer, MetadataReader, ScanHandle, ScanRequest};
pub use scheduler::{
    CancellationToken, IndexJob, IndexScheduler, InteractionMode, JobPriority, SchedulerConfig,
};
use thiserror::Error;
pub use watch::{ChangeHint, WatchService};

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("indexer I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("source root is unavailable: {0}")]
    RootUnavailable(PathBuf),
    #[error("walk failed: {0}")]
    Walk(#[from] walkdir::Error),
    #[error("media path is outside the source root: {0}")]
    PathOutsideRoot(PathBuf),
    #[error("relative path is invalid: {0}")]
    InvalidRelativePath(String),
    #[error("folder policy failed: {0}")]
    Policy(String),
    #[error("scan task failed: {0}")]
    TaskJoin(String),
    #[error("scan cancellation channel is closed")]
    CancellationClosed,
    #[error("scan inventory expected {expected} photos but discovered {discovered}")]
    InventoryMismatch { expected: u64, discovered: u64 },
}
