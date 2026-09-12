use std::env;
use std::net::{AddrParseError, SocketAddr};
use std::path::{Path, PathBuf};
#[cfg(all(debug_assertions, unix))]
use std::sync::Weak;
#[cfg(unix)]
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use photo_app_service::PrevalidatedHostedSource;
#[cfg(unix)]
use photo_core::PrevalidatedSourceKeys;
use photo_core::{LocalStateError, LocalStatePaths};

#[cfg(unix)]
use crate::static_host::PinnedDirectory;
use crate::static_host::{StaticWebRoot, StaticWebRootValidation};

#[derive(Clone, Debug)]
pub struct ServerConfig {
    local: LocalStatePaths,
    bind: SocketAddr,
    source_root: PathBuf,
    allow_original_downloads: bool,
    #[cfg(unix)]
    source_startup: Arc<Mutex<Option<SourceStartupValidation>>>,
    web_root: StaticWebRoot,
}

#[cfg(unix)]
#[derive(Debug)]
struct SourceStartupValidation {
    pinned: PinnedDirectory,
    lifetime: Arc<()>,
}

#[cfg(unix)]
#[derive(Debug)]
pub(crate) struct SourceStartupLease {
    operational: PinnedDirectory,
    lifetime: Arc<()>,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("required environment variable {0} is not set")]
    MissingEnvironment(&'static str),
    #[error("environment variable {0} is not valid Unicode")]
    InvalidEnvironment(&'static str),
    #[error("PHOTO_VIEWER_BIND is not a valid socket address: {0}")]
    InvalidBind(#[from] AddrParseError),
    #[error("PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS must be empty, 0, false, 1, or true")]
    InvalidOriginalDownloads,
    #[error("catalog data and cache directories must not be inside a source root")]
    InsideSourceRoot,
    #[error("configured source root is not a directory")]
    SourceRootNotDirectory,
    #[error("configured source root changed before startup")]
    SourceRootChanged,
    #[error("configured source startup validation is unavailable")]
    SourceStartupUnavailable,
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
        #[cfg(unix)]
        let pinned_source = PinnedDirectory::open(&source_root).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotADirectory {
                ConfigError::SourceRootNotDirectory
            } else {
                ConfigError::Io(error)
            }
        })?;
        #[cfg(unix)]
        let source_root = pinned_source.display_path().to_owned();
        #[cfg(not(unix))]
        let source_root = source_root.canonicalize()?;
        #[cfg(not(unix))]
        if !source_root.is_dir() {
            return Err(ConfigError::SourceRootNotDirectory);
        }
        let web_root = StaticWebRootValidation::capture(web_root)
            .map_err(|_| ConfigError::WebRootUnavailable)?;
        #[cfg(unix)]
        if web_root
            .pinned_directory()
            .overlaps(&pinned_source)
            .map_err(|_| ConfigError::WebRootUnavailable)?
        {
            return Err(ConfigError::WebRootOverlapsSourceRoot);
        }
        if web_root.path().starts_with(&source_root) || source_root.starts_with(web_root.path()) {
            return Err(ConfigError::WebRootOverlapsSourceRoot);
        }
        let local = LocalStatePaths::new(data_dir, cache_dir);
        local
            .validate_source_roots(&[source_root.clone(), web_root.path().to_owned()])
            .map_err(ConfigError::from_local_state)?;
        #[cfg(unix)]
        validate_local_identity(&local, &[&pinned_source, web_root.pinned_directory()])?;
        let web_root = web_root
            .pin()
            .map_err(|_| ConfigError::WebRootUnavailable)?;
        #[cfg(unix)]
        let source_startup = SourceStartupValidation {
            pinned: pinned_source,
            lifetime: Arc::new(()),
        };
        Ok(Self {
            local,
            bind: bind.unwrap_or("127.0.0.1:8080").parse()?,
            source_root,
            allow_original_downloads: false,
            #[cfg(unix)]
            source_startup: Arc::new(Mutex::new(Some(source_startup))),
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
        let allow_original_downloads = match env::var("PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS") {
            Ok(value)
                if value.is_empty() || value == "0" || value.eq_ignore_ascii_case("false") =>
            {
                false
            }
            Ok(value) if value == "1" || value.eq_ignore_ascii_case("true") => true,
            Ok(_) => return Err(ConfigError::InvalidOriginalDownloads),
            Err(env::VarError::NotPresent) => false,
            Err(env::VarError::NotUnicode(_)) => {
                return Err(ConfigError::InvalidEnvironment(
                    "PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS",
                ));
            }
        };
        let mut config = Self::new(data_dir, cache_dir, bind.as_deref(), source_root, web_root)?;
        config.allow_original_downloads = allow_original_downloads;
        Ok(config)
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

    #[cfg(any(not(unix), debug_assertions))]
    pub(crate) fn prepare_source_roots(&self, source_roots: &[PathBuf]) -> Result<(), ConfigError> {
        self.local
            .prepare(source_roots)
            .map_err(ConfigError::from_local_state)
    }

    #[cfg(unix)]
    pub(crate) fn prepare_prevalidated_source_keys(
        &self,
        source_roots: &PrevalidatedSourceKeys,
    ) -> Result<(), ConfigError> {
        self.local
            .prepare_prevalidated_source_keys(source_roots)
            .map_err(ConfigError::from_local_state)
    }

    #[cfg(unix)]
    pub(crate) fn validate_prevalidated_source_keys(
        &self,
        source_roots: &PrevalidatedSourceKeys,
    ) -> Result<(), ConfigError> {
        self.local
            .validate_prevalidated_source_keys(source_roots)
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

    pub const fn allow_original_downloads(&self) -> bool {
        self.allow_original_downloads
    }

    #[doc(hidden)]
    pub fn with_allow_original_downloads(mut self, allow_original_downloads: bool) -> Self {
        self.allow_original_downloads = allow_original_downloads;
        self
    }

    pub fn source_root(&self) -> &Path {
        &self.source_root
    }

    pub fn web_root(&self) -> &Path {
        // Informational only. Static serving and overlap checks use the
        // descriptor retained inside `StaticWebRoot`.
        self.web_root.path()
    }

    pub fn static_web_root(&self) -> StaticWebRoot {
        self.web_root.clone()
    }

    #[cfg(unix)]
    pub(crate) fn take_source_startup(&self) -> Result<SourceStartupLease, ConfigError> {
        let validation = self
            .source_startup
            .lock()
            .map_err(|_| ConfigError::SourceStartupUnavailable)?
            .take()
            .ok_or(ConfigError::SourceStartupUnavailable)?;
        validation.validate(&self.source_root)
    }

    #[cfg(all(debug_assertions, unix))]
    #[doc(hidden)]
    pub fn source_startup_release_probe(&self) -> Option<Weak<()>> {
        self.source_startup
            .lock()
            .ok()?
            .as_ref()
            .map(|validation| Arc::downgrade(&validation.lifetime))
    }
}

#[cfg(unix)]
impl SourceStartupValidation {
    fn validate(self, path: &Path) -> Result<SourceStartupLease, ConfigError> {
        let operational =
            PinnedDirectory::open(path).map_err(|_| ConfigError::SourceRootChanged)?;
        if !self.pinned.same_object_as(&operational) {
            return Err(ConfigError::SourceRootChanged);
        }
        Ok(SourceStartupLease {
            operational,
            lifetime: self.lifetime,
        })
    }
}

#[cfg(unix)]
impl SourceStartupLease {
    pub(crate) fn into_prevalidated_source(
        self,
        operational_path: PathBuf,
        canonical_key: PathBuf,
    ) -> Result<PrevalidatedHostedSource, ConfigError> {
        let SourceStartupLease {
            operational,
            lifetime,
        } = self;
        // SAFETY: `operational` was opened and identity-matched against the
        // descriptor retained from `ServerConfig::new`; both keys are the
        // normalized canonical path captured for that same pinned directory.
        let source = unsafe {
            PrevalidatedHostedSource::from_server_validated_directory(
                operational.into_file(),
                lifetime,
                operational_path,
                canonical_key,
            )
        }
        .map_err(|_| ConfigError::SourceRootChanged)?;
        Ok(source)
    }
}

#[cfg(unix)]
struct DirectoryLocation {
    pinned_parent: PinnedDirectory,
    missing_components: Vec<std::ffi::OsString>,
}

#[cfg(unix)]
impl DirectoryLocation {
    fn inspect(path: &Path) -> Result<Self, std::io::Error> {
        let mut candidate = normalize_absolute(path)?;
        let mut missing_components = Vec::new();
        loop {
            match PinnedDirectory::open(&candidate) {
                Ok(pinned_parent) => {
                    missing_components.reverse();
                    return Ok(Self {
                        pinned_parent,
                        missing_components,
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let Some(component) = candidate.file_name().map(ToOwned::to_owned) else {
                        return Err(error);
                    };
                    missing_components.push(component);
                    if !candidate.pop() {
                        return Err(error);
                    }
                }
                Err(error) => return Err(error),
            }
        }
    }

    fn overlaps(&self, root: &PinnedDirectory) -> Result<bool, std::io::Error> {
        if self.missing_components.is_empty() {
            self.pinned_parent.overlaps(root)
        } else {
            // The first missing component cannot currently contain an existing
            // root. It can only become a descendant of its pinned parent.
            root.identity_is_in_ancestry_of(&self.pinned_parent)
        }
    }
}

#[cfg(unix)]
fn validate_local_identity(
    local: &LocalStatePaths,
    protected_roots: &[&PinnedDirectory],
) -> Result<(), ConfigError> {
    for path in [local.data_dir(), local.cache_dir()] {
        let location = DirectoryLocation::inspect(path)?;
        for root in protected_roots {
            if location.overlaps(root)? {
                return Err(ConfigError::InsideSourceRoot);
            }
        }
    }
    Ok(())
}

#[cfg(unix)]
fn normalize_absolute(path: &Path) -> Result<PathBuf, std::io::Error> {
    let absolute = std::path::absolute(path)?;
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    Ok(normalized)
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
