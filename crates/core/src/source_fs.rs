use std::path::{Path, PathBuf};

pub trait SourceFs: Send + Sync {
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf>;
    fn is_dir(&self, path: &Path) -> bool;
    fn exists(&self, path: &Path) -> bool;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RealSourceFs;

impl SourceFs for RealSourceFs {
    fn canonicalize(&self, path: &Path) -> std::io::Result<PathBuf> {
        std::fs::canonicalize(path)
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
}
