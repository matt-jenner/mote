use std::path::{Path, PathBuf};

use photo_catalog::{Catalog, CatalogError, LibraryRootRecord, NewLibrary};
use photo_domain::{Availability, LibraryId, RelativePathKey};
use thiserror::Error;

use crate::SourceFs;

pub struct LibraryService<F> {
    catalog: Catalog,
    source_fs: F,
    local_state_roots: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceSelection {
    pub library_id: LibraryId,
    pub relative_folder: RelativePathKey,
    pub created_recent_root: bool,
}

#[derive(Debug, Error)]
pub enum AddLibraryError {
    #[error("source path could not be resolved: {0}")]
    Io(#[from] std::io::Error),
    #[error("catalog operation failed: {0}")]
    Catalog(#[from] CatalogError),
    #[error("source is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("source overlaps library {existing_id:?}")]
    Overlaps { existing_id: LibraryId },
    #[error("cataloged root has invalid native encoding: {0}")]
    InvalidCatalogPath(String),
    #[error("selected folder could not be made relative to its library")]
    InvalidSelection,
    #[error("source overlaps a local catalog or cache root")]
    OverlapsLocalState,
}

#[derive(Debug, Error)]
pub enum RelinkError {
    #[error("replacement path could not be resolved: {0}")]
    Io(#[from] std::io::Error),
    #[error("catalog operation failed: {0}")]
    Catalog(#[from] CatalogError),
    #[error("replacement is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("library {0:?} does not exist")]
    LibraryNotFound(LibraryId),
    #[error("replacement root is missing sample files: {missing:?}")]
    VerificationFailed { missing: Vec<PathBuf> },
    #[error("replacement overlaps library {existing_id:?}")]
    Overlaps { existing_id: LibraryId },
    #[error("cataloged root has invalid native encoding: {0}")]
    InvalidCatalogPath(String),
    #[error("replacement overlaps a local catalog or cache root")]
    OverlapsLocalState,
}

impl<F: SourceFs> LibraryService<F> {
    pub fn new(catalog: Catalog, source_fs: F, local_state_roots: Vec<PathBuf>) -> Self {
        Self {
            catalog,
            source_fs,
            local_state_roots,
        }
    }

    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    pub fn add_configured(
        &mut self,
        root: &Path,
        name: &str,
    ) -> Result<LibraryRootRecord, AddLibraryError> {
        let canonical = self.canonical_directory(root)?;
        if self.overlaps_local_state(&canonical) {
            return Err(AddLibraryError::OverlapsLocalState);
        }
        for existing in self.catalog.list_libraries()? {
            let existing_path = existing
                .canonical_root_key
                .to_path_buf()
                .map_err(|error| AddLibraryError::InvalidCatalogPath(error.to_string()))?;
            if paths_overlap(&canonical, &existing_path) {
                return Err(AddLibraryError::Overlaps {
                    existing_id: existing.id,
                });
            }
        }

        let mut library = NewLibrary::configured(name, &canonical);
        library.display_path = root.to_string_lossy().into_owned();
        self.catalog
            .add_library(&library)
            .map_err(AddLibraryError::from)
    }

    pub fn open_recent(&mut self, folder: &Path) -> Result<SourceSelection, AddLibraryError> {
        let canonical = self.canonical_directory(folder)?;
        if self.overlaps_local_state(&canonical) {
            return Err(AddLibraryError::OverlapsLocalState);
        }
        let libraries = self.catalog.list_libraries()?;
        for existing in libraries {
            let existing_path = existing
                .canonical_root_key
                .to_path_buf()
                .map_err(|error| AddLibraryError::InvalidCatalogPath(error.to_string()))?;
            if canonical.starts_with(&existing_path) {
                let relative = canonical
                    .strip_prefix(&existing_path)
                    .map_err(|_| AddLibraryError::InvalidSelection)?;
                return Ok(SourceSelection {
                    library_id: existing.id,
                    relative_folder: RelativePathKey::from_relative_path(relative)
                        .map_err(|_| AddLibraryError::InvalidSelection)?,
                    created_recent_root: false,
                });
            }
        }

        let display_name = canonical
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| canonical.to_string_lossy().into_owned());
        let mut recent = NewLibrary::recent(display_name, &canonical);
        recent.display_path = folder.to_string_lossy().into_owned();
        let library = self.catalog.add_library(&recent)?;
        Ok(SourceSelection {
            library_id: library.id,
            relative_folder: RelativePathKey::from_relative_path(Path::new(""))
                .map_err(|_| AddLibraryError::InvalidSelection)?,
            created_recent_root: true,
        })
    }

    pub fn promote_recent(
        &mut self,
        id: LibraryId,
        name: &str,
    ) -> Result<LibraryRootRecord, CatalogError> {
        self.catalog.promote_library(id, name)
    }

    pub fn mark_offline(&mut self, id: LibraryId) -> Result<(), CatalogError> {
        self.catalog
            .set_library_availability(id, Availability::RootOffline)
    }

    pub fn relink(
        &mut self,
        id: LibraryId,
        replacement: &Path,
        samples: &[RelativePathKey],
    ) -> Result<LibraryRootRecord, RelinkError> {
        if self.catalog.find_library(id)?.is_none() {
            return Err(RelinkError::LibraryNotFound(id));
        }
        let canonical = self
            .source_fs
            .canonicalize(replacement)
            .map_err(RelinkError::Io)?;
        if !self.source_fs.is_dir(&canonical) {
            return Err(RelinkError::NotDirectory(canonical));
        }
        if self.overlaps_local_state(&canonical) {
            return Err(RelinkError::OverlapsLocalState);
        }

        for existing in self.catalog.list_libraries()? {
            if existing.id == id {
                continue;
            }
            let existing_path = existing
                .canonical_root_key
                .to_path_buf()
                .map_err(|error| RelinkError::InvalidCatalogPath(error.to_string()))?;
            if paths_overlap(&canonical, &existing_path) {
                return Err(RelinkError::Overlaps {
                    existing_id: existing.id,
                });
            }
        }

        let mut missing = Vec::new();
        for sample in samples {
            let relative = sample
                .to_path_buf()
                .map_err(|error| RelinkError::InvalidCatalogPath(error.to_string()))?;
            if !self.source_fs.exists(&canonical.join(&relative)) {
                missing.push(relative);
            }
        }
        if !missing.is_empty() {
            return Err(RelinkError::VerificationFailed { missing });
        }

        self.catalog
            .relink_library(id, &canonical, replacement)
            .map_err(RelinkError::from)
    }

    fn canonical_directory(&self, path: &Path) -> Result<PathBuf, AddLibraryError> {
        let canonical = self.source_fs.canonicalize(path)?;
        if !self.source_fs.is_dir(&canonical) {
            return Err(AddLibraryError::NotDirectory(canonical));
        }
        Ok(canonical)
    }

    fn overlaps_local_state(&self, source: &Path) -> bool {
        self.local_state_roots
            .iter()
            .any(|state| paths_overlap(source, state))
    }
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left.starts_with(right) || right.starts_with(left)
}
