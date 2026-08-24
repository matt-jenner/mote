use std::path::{Path, PathBuf};

use photo_core::{LocalStateError, LocalStatePaths};

#[derive(Clone, Debug)]
pub struct AppConfig {
    local: LocalStatePaths,
}

impl AppConfig {
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            local: LocalStatePaths::new(data_dir, cache_dir),
        }
    }

    pub fn data_dir(&self) -> &Path {
        self.local.data_dir()
    }

    pub fn cache_dir(&self) -> &Path {
        self.local.cache_dir()
    }

    pub fn catalog_path(&self) -> PathBuf {
        self.local.catalog_path()
    }

    pub(crate) fn validate_source_roots(&self, roots: &[PathBuf]) -> Result<(), LocalStateError> {
        self.local.validate_source_roots(roots)
    }

    pub(crate) fn prepare(&self, roots: &[PathBuf]) -> Result<(), LocalStateError> {
        self.local.prepare(roots)
    }
}
