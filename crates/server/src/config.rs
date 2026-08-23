use std::env;
use std::net::{AddrParseError, SocketAddr};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    data_dir: PathBuf,
    cache_dir: PathBuf,
    bind: SocketAddr,
    source_roots: Vec<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("required environment variable {0} is not set")]
    MissingEnvironment(&'static str),
    #[error("environment variable {0} is not valid Unicode")]
    InvalidEnvironment(&'static str),
    #[error("PHOTO_VIEWER_BIND is not a valid socket address: {0}")]
    InvalidBind(#[from] AddrParseError),
    #[error("catalog data and cache directories must not be inside a source root")]
    InsideSourceRoot,
    #[error("configuration filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
}

impl ServerConfig {
    pub fn new(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        bind: Option<&str>,
        source_roots: Vec<PathBuf>,
    ) -> Result<Self, ConfigError> {
        Ok(Self {
            data_dir,
            cache_dir,
            bind: bind.unwrap_or("127.0.0.1:8080").parse()?,
            source_roots,
        })
    }

    pub fn from_env(source_roots: Vec<PathBuf>) -> Result<Self, ConfigError> {
        let data_dir = required_path("PHOTO_VIEWER_DATA_DIR")?;
        let cache_dir = required_path("PHOTO_VIEWER_CACHE_DIR")?;
        let bind = match env::var("PHOTO_VIEWER_BIND") {
            Ok(value) => Some(value),
            Err(env::VarError::NotPresent) => None,
            Err(env::VarError::NotUnicode(_)) => {
                return Err(ConfigError::InvalidEnvironment("PHOTO_VIEWER_BIND"));
            }
        };
        Self::new(data_dir, cache_dir, bind.as_deref(), source_roots)
    }

    pub fn prepare(&self) -> Result<(), ConfigError> {
        self.validate_source_roots(&self.source_roots)?;
        create_private_directory(&self.data_dir)?;
        create_private_directory(&self.cache_dir)?;
        Ok(())
    }

    pub fn validate_source_roots(&self, source_roots: &[PathBuf]) -> Result<(), ConfigError> {
        let data = resolve_for_comparison(&self.data_dir)?;
        let cache = resolve_for_comparison(&self.cache_dir)?;
        let catalog = resolve_for_comparison(&self.catalog_path())?;
        for source in source_roots {
            let source = resolve_for_comparison(source)?;
            if paths_overlap(&data, &source)
                || paths_overlap(&cache, &source)
                || paths_overlap(&catalog, &source)
            {
                return Err(ConfigError::InsideSourceRoot);
            }
        }
        Ok(())
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

    pub const fn bind(&self) -> SocketAddr {
        self.bind
    }
}

fn required_path(name: &'static str) -> Result<PathBuf, ConfigError> {
    match env::var(name) {
        Ok(value) => Ok(PathBuf::from(value)),
        Err(env::VarError::NotPresent) => Err(ConfigError::MissingEnvironment(name)),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidEnvironment(name)),
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
