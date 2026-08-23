use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use photo_catalog::Catalog;

use crate::CacheError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CacheWrite {
    pub relative_path: PathBuf,
    pub size_bytes: u64,
    pub reused: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CacheReconcileReport {
    pub partial_files_removed: u64,
    pub missing_rows_removed: u64,
}

#[derive(Clone, Debug)]
pub struct CacheWriter {
    root: PathBuf,
}

impl CacheWriter {
    pub fn new(root: &Path) -> Result<Self, CacheError> {
        std::fs::create_dir_all(root)?;
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    pub fn write_atomic<F>(
        &self,
        relative_path: PathBuf,
        write: F,
    ) -> Result<CacheWrite, CacheError>
    where
        F: FnOnce(&mut File) -> std::io::Result<()>,
    {
        let final_path = self.resolve(&relative_path)?;
        if final_path.is_file() {
            return Ok(CacheWrite {
                relative_path,
                size_bytes: final_path.metadata()?.len(),
                reused: true,
            });
        }

        let parent = final_path.parent().ok_or(CacheError::PathEscape)?;
        self.create_safe_directories(parent)?;
        let file_name = final_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(CacheError::PathEscape)?;
        let partial_path = parent.join(format!("{file_name}.partial-{}", uuid::Uuid::new_v4()));
        let mut partial = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial_path)?;

        if let Err(error) = write(&mut partial) {
            drop(partial);
            let _ = std::fs::remove_file(&partial_path);
            return Err(CacheError::Write(error));
        }
        if let Err(error) = partial.flush().and_then(|()| partial.sync_all()) {
            drop(partial);
            let _ = std::fs::remove_file(&partial_path);
            return Err(CacheError::Write(error));
        }
        let size_bytes = partial.metadata()?.len();
        drop(partial);

        if final_path.exists() {
            std::fs::remove_file(&partial_path)?;
            return Ok(CacheWrite {
                relative_path,
                size_bytes: final_path.metadata()?.len(),
                reused: true,
            });
        }

        if let Err(error) = std::fs::rename(&partial_path, &final_path) {
            let _ = std::fs::remove_file(&partial_path);
            return Err(CacheError::Write(error));
        }
        Ok(CacheWrite {
            relative_path,
            size_bytes,
            reused: false,
        })
    }

    pub fn reconcile_catalog(
        &self,
        catalog: &mut Catalog,
    ) -> Result<CacheReconcileReport, CacheError> {
        let mut partial_files_removed = 0_u64;
        for entry in walkdir::WalkDir::new(&self.root).follow_links(false) {
            let entry = entry.map_err(|error| {
                CacheError::Io(
                    error
                        .into_io_error()
                        .unwrap_or_else(|| std::io::Error::other("could not walk cache directory")),
                )
            })?;
            if entry.file_type().is_file()
                && entry.file_name().to_string_lossy().contains(".partial-")
            {
                std::fs::remove_file(entry.path())?;
                partial_files_removed += 1;
            }
        }

        let missing = catalog
            .all_derivatives()?
            .into_iter()
            .filter_map(|record| {
                self.resolve(&record.relative_cache_path)
                    .map(|path| (!path.is_file()).then_some(record.id))
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let missing_rows_removed = catalog.delete_derivatives(&missing)?;
        Ok(CacheReconcileReport {
            partial_files_removed,
            missing_rows_removed: u64::try_from(missing_rows_removed)
                .map_err(|_| CacheError::SizeOutOfRange)?,
        })
    }

    pub(crate) fn resolve(&self, relative_path: &Path) -> Result<PathBuf, CacheError> {
        validate_relative(relative_path)?;
        Ok(self.root.join(relative_path))
    }

    fn create_safe_directories(&self, parent: &Path) -> Result<(), CacheError> {
        let relative = parent
            .strip_prefix(&self.root)
            .map_err(|_| CacheError::PathEscape)?;
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component.as_os_str());
            match std::fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(CacheError::PathEscape);
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    std::fs::create_dir(&current)?;
                }
                Err(error) => return Err(error.into()),
            }
        }
        Ok(())
    }
}

pub(crate) fn validate_relative(path: &Path) -> Result<(), CacheError> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(CacheError::PathEscape);
    }
    Ok(())
}
