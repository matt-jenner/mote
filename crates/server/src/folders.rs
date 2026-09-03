use std::fs;
use std::path::{Component, Path, PathBuf};

use photo_domain::MediaKind;
use serde::Serialize;

pub use photo_app_service::FolderBreadcrumb;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub path: String,
    pub breadcrumbs: Vec<FolderBreadcrumb>,
    pub children: Vec<FolderEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_count: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FolderError {
    #[error("invalid relative folder path")]
    InvalidPath,
    #[error("folder resolves outside the source root")]
    OutsideRoot,
    #[error("folder is unavailable")]
    Unavailable,
    #[error("path is not a directory")]
    NotDirectory,
    #[error("folder is unreadable")]
    Unreadable,
}

#[derive(Clone, Debug)]
pub struct ContainedFolderRoot {
    root: PathBuf,
}

impl ContainedFolderRoot {
    pub fn new(root: PathBuf) -> Result<Self, FolderError> {
        let root = root.canonicalize().map_err(map_resolve_error)?;
        if !root.is_dir() {
            return Err(FolderError::NotDirectory);
        }
        Ok(Self { root })
    }

    #[cfg(unix)]
    pub(crate) fn from_prevalidated_operational_path(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn resolve(&self, relative: &str) -> Result<PathBuf, FolderError> {
        let canonical = self.resolve_path(relative)?;
        fs::read_dir(&canonical).map_err(map_read_error)?;
        Ok(canonical)
    }

    fn resolve_path(&self, relative: &str) -> Result<PathBuf, FolderError> {
        validate_relative(relative)?;
        let joined = if relative.is_empty() {
            self.root.clone()
        } else {
            self.root.join(relative)
        };
        let canonical = joined.canonicalize().map_err(map_resolve_error)?;
        if !canonical.starts_with(&self.root) {
            return Err(FolderError::OutsideRoot);
        }
        if !canonical.is_dir() {
            return Err(FolderError::NotDirectory);
        }
        Ok(canonical)
    }

    pub fn list(&self, relative: &str) -> Result<FolderListing, FolderError> {
        self.list_internal(relative, false)
    }

    pub fn list_with_image_count(&self, relative: &str) -> Result<FolderListing, FolderError> {
        self.list_internal(relative, true)
    }

    fn list_internal(
        &self,
        relative: &str,
        include_image_count: bool,
    ) -> Result<FolderListing, FolderError> {
        let directory = self.resolve_path(relative)?;
        let mut entries = Vec::new();
        let mut image_count = 0;
        let read_dir = fs::read_dir(&directory).map_err(map_read_error)?;
        for entry in read_dir {
            let entry = match entry {
                Ok(entry) => entry,
                Err(_) => continue,
            };
            let kind = match entry.file_type() {
                Ok(kind) => kind,
                Err(_) => continue,
            };
            if kind.is_file() {
                if include_image_count
                    && MediaKind::from_path(&entry.path())
                        .is_some_and(|kind| kind != MediaKind::Video)
                {
                    image_count += 1;
                }
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let child_relative = join_relative(relative, &name);
            if kind.is_dir() {
                entries.push(FolderEntry {
                    name,
                    path: child_relative,
                });
                continue;
            }
            if !kind.is_symlink() {
                continue;
            }
            let child = match self.resolve_path(&child_relative) {
                Ok(child) => child,
                Err(FolderError::OutsideRoot) => continue,
                Err(
                    FolderError::Unavailable | FolderError::NotDirectory | FolderError::Unreadable,
                ) => continue,
                Err(FolderError::InvalidPath) => continue,
            };
            if child.is_dir() {
                entries.push(FolderEntry {
                    name,
                    path: child_relative,
                });
            }
        }
        entries.sort_by(|left, right| {
            left.name
                .to_lowercase()
                .cmp(&right.name.to_lowercase())
                .then_with(|| left.name.cmp(&right.name))
        });
        Ok(FolderListing {
            path: relative.to_owned(),
            breadcrumbs: breadcrumbs(relative),
            children: entries,
            image_count: include_image_count.then_some(image_count),
        })
    }

    pub(crate) fn is_available(&self) -> bool {
        fs::read_dir(&self.root).is_ok()
    }

    pub(crate) fn canonical_root(&self) -> &Path {
        &self.root
    }
}

fn validate_relative(relative: &str) -> Result<(), FolderError> {
    if relative.contains('\0') || relative.contains('\\') || relative.starts_with('/') {
        return Err(FolderError::InvalidPath);
    }
    if relative.is_empty() {
        return Ok(());
    }
    if relative.split('/').any(|segment| segment.is_empty()) {
        return Err(FolderError::InvalidPath);
    }
    for component in Path::new(relative).components() {
        match component {
            Component::Normal(_) => {}
            Component::CurDir
            | Component::ParentDir
            | Component::RootDir
            | Component::Prefix(_) => return Err(FolderError::InvalidPath),
        }
    }
    Ok(())
}

fn breadcrumbs(relative: &str) -> Vec<FolderBreadcrumb> {
    let mut path = String::new();
    relative
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(|name| {
            if !path.is_empty() {
                path.push('/');
            }
            path.push_str(name);
            FolderBreadcrumb {
                name: name.to_owned(),
                path: path.clone(),
            }
        })
        .collect()
}

fn join_relative(parent: &str, child: &str) -> String {
    if parent.is_empty() {
        child.to_owned()
    } else {
        format!("{parent}/{child}")
    }
}

fn map_resolve_error(error: std::io::Error) -> FolderError {
    match error.kind() {
        std::io::ErrorKind::NotFound => FolderError::Unavailable,
        std::io::ErrorKind::PermissionDenied => FolderError::Unreadable,
        _ => FolderError::Unavailable,
    }
}

fn map_read_error(error: std::io::Error) -> FolderError {
    match error.kind() {
        std::io::ErrorKind::PermissionDenied => FolderError::Unreadable,
        std::io::ErrorKind::NotFound => FolderError::Unavailable,
        _ => FolderError::Unreadable,
    }
}
