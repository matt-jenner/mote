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

/// Server-internal capability for source identity keys whose provenance was
/// validated outside the path-based local-state layer.
///
/// Safe callers cannot construct this type. Possessing it only permits
/// no-source-I/O overlap checks; it does not grant filesystem access.
#[cfg(feature = "server-internal-prevalidated-source")]
#[doc(hidden)]
#[derive(Debug)]
pub struct PrevalidatedSourceKeys {
    keys: Vec<PathBuf>,
}

#[cfg(feature = "server-internal-prevalidated-source")]
impl PrevalidatedSourceKeys {
    /// Constructs a source-key capability at an audited identity boundary.
    ///
    /// # Safety
    ///
    /// The caller must ensure every input is an established source identity
    /// key for which lexical overlap comparison is appropriate. This function
    /// verifies that inputs are absolute and stores only normalized keys, but
    /// cannot prove their descriptor or catalog provenance.
    #[doc(hidden)]
    pub unsafe fn from_validated_identity_keys(keys: Vec<PathBuf>) -> std::io::Result<Self> {
        let keys = keys
            .into_iter()
            .map(|key| {
                if !key.is_absolute() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "prevalidated source key is invalid",
                    ));
                }
                normalize_prevalidated_source_key(&key)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { keys })
    }
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
            let source = match resolve_for_comparison(source) {
                Ok(source) => source,
                Err(error) if is_unavailable_source_error(&error) => continue,
                Err(error) => return Err(error.into()),
            };
            if paths_overlap(&data, &source)
                || paths_overlap(&cache, &source)
                || paths_overlap(&catalog, &source)
            {
                return Err(LocalStateError::InsideSourceRoot);
            }
        }
        Ok(())
    }

    /// Validates source identities established by an explicit capability.
    /// Source keys are never reopened here.
    #[cfg(feature = "server-internal-prevalidated-source")]
    pub fn validate_prevalidated_source_keys(
        &self,
        sources: &PrevalidatedSourceKeys,
    ) -> Result<(), LocalStateError> {
        let data = resolve_for_comparison(&self.data_dir)?;
        let cache = resolve_for_comparison(&self.cache_dir)?;
        let catalog = resolve_for_comparison(&self.catalog_path())?;
        for source in &sources.keys {
            if paths_overlap(&data, source)
                || paths_overlap(&cache, source)
                || paths_overlap(&catalog, source)
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
    #[cfg(feature = "server-internal-prevalidated-source")]
    pub fn prepare_prevalidated_source_keys(
        &self,
        sources: &PrevalidatedSourceKeys,
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

fn is_unavailable_source_error(error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        std::io::ErrorKind::NotFound
            | std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::Other
    ) {
        return true;
    }

    #[cfg(target_os = "linux")]
    return matches!(error.raw_os_error(), Some(19 | 107 | 116));

    #[cfg(not(target_os = "linux"))]
    false
}

#[cfg(test)]
mod tests {
    use super::is_unavailable_source_error;

    #[cfg(unix)]
    #[test]
    fn disconnected_source_errors_are_treated_as_unavailable() {
        let error = std::io::Error::from_raw_os_error(19);

        assert!(is_unavailable_source_error(&error));
    }
}
