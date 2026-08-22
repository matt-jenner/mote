use std::io;
use std::path::{Path, PathBuf};

use photo_catalog::{Catalog, CatalogError};
use photo_domain::LibraryId;
use thiserror::Error;

use crate::discover::discover_asset;

pub trait ReconcileSource: Send + Sync {
    fn media_paths(&self, root: &Path) -> io::Result<Vec<PathBuf>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct RealReconcileSource;

impl ReconcileSource for RealReconcileSource {
    fn media_paths(&self, root: &Path) -> io::Result<Vec<PathBuf>> {
        std::fs::read_dir(root)?;
        let mut paths = Vec::new();
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            let entry = entry.map_err(|error| {
                error.into_io_error().unwrap_or_else(|| {
                    io::Error::other("directory walk failed without an I/O cause")
                })
            })?;
            if entry.file_type().is_file() {
                paths.push(entry.into_path());
            }
        }
        Ok(paths)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReconcileOutcome {
    Completed {
        generation: u64,
        observed: u64,
        marked_missing: u64,
    },
    RootOffline {
        retained_assets: u64,
    },
}

#[derive(Debug, Error)]
pub enum ReconcileError {
    #[error("catalog reconciliation failed: {0}")]
    Catalog(#[from] CatalogError),
}

pub struct Reconciler<S> {
    catalog: Catalog,
    library_id: LibraryId,
    root: PathBuf,
    source: S,
}

impl<S: ReconcileSource> Reconciler<S> {
    pub fn new(catalog: Catalog, library_id: LibraryId, root: PathBuf, source: S) -> Self {
        Self {
            catalog,
            library_id,
            root,
            source,
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn catalog_mut(&mut self) -> &mut Catalog {
        &mut self.catalog
    }

    pub fn into_catalog(self) -> Catalog {
        self.catalog
    }

    pub async fn run(&mut self) -> Result<ReconcileOutcome, ReconcileError> {
        let paths = match self.source.media_paths(&self.root) {
            Ok(paths) => paths,
            Err(error) => {
                tracing::warn!(
                    library_id = %self.library_id.as_uuid(),
                    error = %error,
                    "source root is offline"
                );
                let retained_assets = self.catalog.mark_root_offline(self.library_id)?;
                return Ok(ReconcileOutcome::RootOffline { retained_assets });
            }
        };

        let generation = self.catalog.begin_generation(self.library_id)?;
        let mut assets = Vec::new();
        for path in paths {
            match discover_asset(&self.root, &path, self.library_id) {
                Ok(Some(asset)) => assets.push(asset.asset),
                Ok(None) => {}
                Err(error) => {
                    tracing::warn!(
                        library_id = %self.library_id.as_uuid(),
                        error = %error,
                        "source changed during reconciliation"
                    );
                    let retained_assets = self.catalog.mark_root_offline(self.library_id)?;
                    return Ok(ReconcileOutcome::RootOffline { retained_assets });
                }
            }
        }
        for batch in assets.chunks(500) {
            self.catalog
                .record_generation_assets(self.library_id, generation, batch)?;
        }
        let completion = self
            .catalog
            .complete_generation(self.library_id, generation)?;
        Ok(ReconcileOutcome::Completed {
            generation,
            observed: assets.len() as u64,
            marked_missing: completion.marked_missing,
        })
    }
}
