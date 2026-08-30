use std::env;
use std::net::{AddrParseError, SocketAddr};
use std::path::{Path, PathBuf};

use photo_core::{LocalStateError, LocalStatePaths};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    local: LocalStatePaths,
    bind: SocketAddr,
    source_root: PathBuf,
    web_root: PathBuf,
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
    #[error("configured source root is not a directory")]
    SourceRootNotDirectory,
    #[error("configured web root is unavailable")]
    WebRootUnavailable,
    #[error("configured web root must not overlap the source root")]
    WebRootOverlapsSourceRoot,
    #[error("configuration filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
}

impl ServerConfig {
    pub fn new(
        data_dir: PathBuf,
        cache_dir: PathBuf,
        bind: Option<&str>,
        source_root: PathBuf,
        web_root: PathBuf,
    ) -> Result<Self, ConfigError> {
        let source_root = source_root.canonicalize()?;
        if !source_root.is_dir() {
            return Err(ConfigError::SourceRootNotDirectory);
        }
        let web_root = web_root
            .canonicalize()
            .map_err(|_| ConfigError::WebRootUnavailable)?;
        if !web_root.is_dir() {
            return Err(ConfigError::WebRootUnavailable);
        }
        if web_root.starts_with(&source_root) || source_root.starts_with(&web_root) {
            return Err(ConfigError::WebRootOverlapsSourceRoot);
        }
        let local = LocalStatePaths::new(data_dir, cache_dir);
        local
            .validate_source_roots(&[source_root.clone(), web_root.clone()])
            .map_err(ConfigError::from_local_state)?;
        Ok(Self {
            local,
            bind: bind.unwrap_or("127.0.0.1:8080").parse()?,
            source_root,
            web_root,
        })
    }

    pub fn from_env() -> Result<Self, ConfigError> {
        let data_dir = required_path("PHOTO_VIEWER_DATA_DIR")?;
        let cache_dir = required_path("PHOTO_VIEWER_CACHE_DIR")?;
        let source_root = required_path("PHOTO_VIEWER_SOURCE_ROOT")?;
        let bind = match env::var("PHOTO_VIEWER_BIND") {
            Ok(value) => Some(value),
            Err(env::VarError::NotPresent) => None,
            Err(env::VarError::NotUnicode(_)) => {
                return Err(ConfigError::InvalidEnvironment("PHOTO_VIEWER_BIND"));
            }
        };
        let web_root = match env::var("PHOTO_VIEWER_WEB_ROOT") {
            Ok(value) => PathBuf::from(value),
            Err(env::VarError::NotPresent) => PathBuf::from("/app/web"),
            Err(env::VarError::NotUnicode(_)) => {
                return Err(ConfigError::InvalidEnvironment("PHOTO_VIEWER_WEB_ROOT"));
            }
        };
        Self::new(data_dir, cache_dir, bind.as_deref(), source_root, web_root)
    }

    pub fn prepare(&self) -> Result<(), ConfigError> {
        self.local
            .prepare(std::slice::from_ref(&self.source_root))
            .map_err(ConfigError::from_local_state)
    }

    pub fn validate_source_roots(&self, source_roots: &[PathBuf]) -> Result<(), ConfigError> {
        self.local
            .validate_source_roots(source_roots)
            .map_err(ConfigError::from_local_state)
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

    pub const fn bind(&self) -> SocketAddr {
        self.bind
    }

    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    pub fn web_root(&self) -> &Path {
        &self.web_root
    }
}

impl ConfigError {
    fn from_local_state(error: LocalStateError) -> Self {
        match error {
            LocalStateError::InsideSourceRoot => Self::InsideSourceRoot,
            LocalStateError::Io(error) => Self::Io(error),
        }
    }
}

fn required_path(name: &'static str) -> Result<PathBuf, ConfigError> {
    match env::var(name) {
        Ok(value) => Ok(PathBuf::from(value)),
        Err(env::VarError::NotPresent) => Err(ConfigError::MissingEnvironment(name)),
        Err(env::VarError::NotUnicode(_)) => Err(ConfigError::InvalidEnvironment(name)),
    }
}
