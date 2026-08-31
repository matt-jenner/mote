use std::path::{Component, Path, PathBuf};

use thiserror::Error;

#[derive(Clone, Debug)]
pub struct LocalStatePaths {
    data_dir: PathBuf,
    cache_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum LocalStateError {
    #[error("catalog data and cache directories must not overlap a source root")]
    InsideSourceRoot,
    #[error("local-state filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
}

impl LocalStatePaths {
    pub fn new(data_dir: PathBuf, cache_dir: PathBuf) -> Self {
        Self {
            data_dir,
            cache_dir,
        }
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn catalog_path(&self) -> PathBuf {
        self.data_dir.join("catalog.sqlite")
    }

    pub fn validate_source_roots(&self, sources: &[PathBuf]) -> Result<(), LocalStateError> {
        let data = resolve_for_comparison(&self.data_dir)?;
        let cache = resolve_for_comparison(&self.cache_dir)?;
        let catalog = resolve_for_comparison(&self.catalog_path())?;
        for source in sources {
            let source = resolve_for_comparison(source)?;
            if paths_overlap(&data, &source)
                || paths_overlap(&cache, &source)
                || paths_overlap(&catalog, &source)
            {
                return Err(LocalStateError::InsideSourceRoot);
            }
        }
        Ok(())
    }

    /// Validates source identities already established by a trusted caller.
    /// Source keys are normalized lexically and are never reopened here.
    pub fn validate_prevalidated_source_keys(
        &self,
        sources: &[PathBuf],
    ) -> Result<(), LocalStateError> {
        let data = resolve_for_comparison(&self.data_dir)?;
        let cache = resolve_for_comparison(&self.cache_dir)?;
        let catalog = resolve_for_comparison(&self.catalog_path())?;
        for source in sources {
            let source = normalize_prevalidated_source_key(source)?;
            if paths_overlap(&data, &source)
                || paths_overlap(&cache, &source)
                || paths_overlap(&catalog, &source)
            {
                return Err(LocalStateError::InsideSourceRoot);
            }
        }
        Ok(())
    }

    pub fn prepare(&self, sources: &[PathBuf]) -> Result<(), LocalStateError> {
        self.validate_source_roots(sources)?;
        create_private_directory(&self.data_dir)?;
        create_private_directory(&self.cache_dir)?;
        Ok(())
    }

    /// Prepares local state after source identities were pinned and validated
    /// by a trusted startup boundary. This method performs no source I/O.
    pub fn prepare_prevalidated_source_keys(
        &self,
        sources: &[PathBuf],
    ) -> Result<(), LocalStateError> {
        self.validate_prevalidated_source_keys(sources)?;
        create_private_directory(&self.data_dir)?;
        create_private_directory(&self.cache_dir)?;
        Ok(())
    }
}

fn normalize_absolute(path: &Path) -> Result<PathBuf, std::io::Error> {
    let absolute = std::path::absolute(path)?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
}

/// Lexically normalizes an already-validated source identity without touching
/// the source filesystem.
pub fn normalize_prevalidated_source_key(path: &Path) -> Result<PathBuf, std::io::Error> {
    let normalized = normalize_absolute(path)?;
    #[cfg(target_os = "macos")]
    for (alias, canonical) in [
        (Path::new("/var"), Path::new("/private/var")),
        (Path::new("/tmp"), Path::new("/private/tmp")),
        (Path::new("/etc"), Path::new("/private/etc")),
    ] {
        if let Ok(remainder) = normalized.strip_prefix(alias) {
            return Ok(canonical.join(remainder));
        }
    }
    Ok(normalized)
}

fn resolve_for_comparison(path: &Path) -> Result<PathBuf, std::io::Error> {
    let absolute = normalize_absolute(path)?;
    let mut ancestor = absolute.clone();
    let mut missing = Vec::new();
    loop {
        match ancestor.canonicalize() {
            Ok(mut resolved) => {
                for component in missing.iter().rev() {
                    resolved.push(component);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(component) = ancestor.file_name().map(ToOwned::to_owned) else {
                    return Err(error);
                };
                missing.push(component);
                if !ancestor.pop() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(not(windows))]
fn path_starts_with(path: &Path, root: &Path) -> bool {
    path.starts_with(root)
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    path_starts_with(left, right) || path_starts_with(right, left)
}

#[cfg(windows)]
fn path_starts_with(path: &Path, root: &Path) -> bool {
    let mut path_components = path.components();
    root.components().all(|root_component| {
        path_components.next().is_some_and(|path_component| {
            path_component
                .as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&root_component.as_os_str().to_string_lossy())
        })
    })
}

fn create_private_directory(path: &Path) -> Result<(), std::io::Error> {
    std::fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
