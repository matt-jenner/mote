mod catalog_writer;
mod discover;
mod events;
mod scanner;
mod scheduler;

use std::path::PathBuf;

pub use catalog_writer::CatalogWriter;
pub use discover::find_sidecar;
pub use events::{IndexEvent, ScanSummary};
pub use scanner::{Indexer, MetadataReader, ScanHandle, ScanRequest};
pub use scheduler::{
    CancellationToken, IndexJob, IndexScheduler, InteractionMode, JobPriority, SchedulerConfig,
};
use thiserror::Error;

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
}
