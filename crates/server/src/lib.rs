mod api;
mod config;
mod folders;
mod health;
mod static_host;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{ConnectInfo, State};
use axum::routing::{any, get};
#[cfg(unix)]
use photo_app_service::AppConfig;
use photo_app_service::{AppServiceError, GalleryEngine};
use photo_cache::{CacheReconcileReport, CacheWriter};
use photo_catalog::Catalog;
#[cfg(unix)]
use photo_core::PrevalidatedSourceKeys;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::TraceLayer;

pub use config::{ConfigError, ServerConfig};
pub use folders::{ContainedFolderRoot, FolderBreadcrumb, FolderEntry, FolderError, FolderListing};
pub use health::{
    ComponentHealth, ComponentStatus, HealthReport, HealthStatus, SourceHealthCounts,
};
pub use static_host::StaticWebRoot;

#[doc(hidden)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceStartupTestStage {
    BeforeOperationalValidation,
    AfterOperationalValidation,
    BeforeFolderConstruction,
    AfterFolderConstruction,
    BeforeGalleryConstruction,
    AfterGalleryConstruction,
}

#[derive(Clone)]
pub struct AppState {
    pub(crate) catalog: Arc<Mutex<Catalog>>,
    pub(crate) cache_root: Arc<PathBuf>,
    pub(crate) folder_root: Option<Arc<ContainedFolderRoot>>,
    pub(crate) gallery: Option<Arc<GalleryEngine>>,
    pub(crate) accent_color: Option<String>,
    pub(crate) allow_original_downloads: bool,
    #[cfg(unix)]
    pub(crate) original_root: Option<Arc<static_host::PinnedDirectory>>,
}

#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("server configuration failed: {0}")]
    Config(#[from] ConfigError),
    #[error("catalog startup failed: {0}")]
    Catalog(#[from] photo_catalog::CatalogError),
    #[error("cache startup failed: {0}")]
    Cache(#[from] photo_cache::CacheError),
    #[error("cataloged source root has invalid native encoding")]
    InvalidCatalogPath,
    #[error("source root startup failed: {0}")]
    Folder(#[from] FolderError),
    #[error("gallery startup failed: {0}")]
    Gallery(#[from] AppServiceError),
}

impl AppState {
    pub(crate) async fn record_source_root_listing(&self, available: bool) {
        let Some(folder_root) = self.folder_root.as_ref() else {
            return;
        };
        let canonical_root = photo_domain::NativePathKey::from_path(folder_root.canonical_root());
        let library = {
            let Ok(catalog) = self.catalog.lock() else {
                tracing::warn!("source availability could not acquire the catalog");
                return;
            };
            let Ok(libraries) = catalog.list_libraries() else {
                tracing::warn!("source availability could not read the catalog");
                return;
            };
            libraries
                .into_iter()
                .find(|library| library.canonical_root_key == canonical_root)
        };
        let Some(library) = library else {
            return;
        };
        if !available && let Some(gallery) = &self.gallery {
            match gallery
                .mark_hosted_library_root_unavailable(library.id)
                .await
            {
                Ok(true) => return,
                Ok(false) => {}
                Err(error) => {
                    tracing::warn!(%error, "source availability could not be persisted");
                    return;
                }
            }
        }
        let Ok(mut catalog) = self.catalog.lock() else {
            tracing::warn!("source availability could not acquire the catalog");
            return;
        };
        let result = if available {
            catalog.set_library_availability(library.id, photo_domain::Availability::Available)
        } else {
            catalog.mark_root_offline(library.id).map(|_| ())
        };
        match result {
            Ok(()) => {}
            Err(error) => {
                tracing::warn!(%error, "source availability could not be persisted");
            }
        }
    }

    pub fn new(catalog: Catalog, cache_root: PathBuf) -> Self {
        Self {
            catalog: Arc::new(Mutex::new(catalog)),
            cache_root: Arc::new(cache_root),
            folder_root: None,
            gallery: None,
            accent_color: None,
            allow_original_downloads: false,
            #[cfg(unix)]
            original_root: None,
        }
    }

    pub fn new_with_source_root(
        catalog: Catalog,
        cache_root: PathBuf,
        source_root: PathBuf,
    ) -> Result<Self, FolderError> {
        Ok(Self {
            catalog: Arc::new(Mutex::new(catalog)),
            cache_root: Arc::new(cache_root),
            folder_root: Some(Arc::new(ContainedFolderRoot::new(source_root)?)),
            gallery: None,
            accent_color: None,
            allow_original_downloads: false,
            #[cfg(unix)]
            original_root: None,
        })
    }

    pub fn open(config: &ServerConfig) -> Result<(Self, CacheReconcileReport), StartupError> {
        #[cfg(unix)]
        {
            Self::open_hosted_inner(config, |_| {})
        }
        #[cfg(not(unix))]
        {
            Self::open_source_disabled(config)
        }
    }

    #[cfg(all(debug_assertions, unix))]
    #[doc(hidden)]
    pub fn open_with_source_startup_hook<F>(
        config: &ServerConfig,
        before_operational_validation: F,
    ) -> Result<(Self, CacheReconcileReport), StartupError>
    where
        F: FnOnce(),
    {
        let mut before_operational_validation = Some(before_operational_validation);
        Self::open_hosted_inner(config, move |stage| {
            if stage == SourceStartupTestStage::BeforeOperationalValidation
                && let Some(hook) = before_operational_validation.take()
            {
                hook();
            }
        })
    }

    #[cfg(all(debug_assertions, unix))]
    #[doc(hidden)]
    pub fn open_with_source_construction_hook<F>(
        config: &ServerConfig,
        hook: F,
    ) -> Result<(Self, CacheReconcileReport), StartupError>
    where
        F: FnMut(SourceStartupTestStage),
    {
        Self::open_hosted_inner(config, hook)
    }

    #[cfg(unix)]
    fn open_hosted_inner<F>(
        config: &ServerConfig,
        mut source_startup_hook: F,
    ) -> Result<(Self, CacheReconcileReport), StartupError>
    where
        F: FnMut(SourceStartupTestStage),
    {
        source_startup_hook(SourceStartupTestStage::BeforeOperationalValidation);
        let source_startup = config.take_source_startup()?;
        source_startup_hook(SourceStartupTestStage::AfterOperationalValidation);
        let mut preflight_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        preflight_roots.push(config.source_root().to_owned());
        // SAFETY: catalog roots are typed canonical identity keys and the
        // configured root is tied to the live startup lease above. They are
        // used only for no-source-I/O lexical overlap checks.
        let preflight_keys =
            unsafe { PrevalidatedSourceKeys::from_validated_identity_keys(preflight_roots) }
                .map_err(|_| StartupError::InvalidCatalogPath)?;
        config.validate_prevalidated_source_keys(&preflight_keys)?;
        config.prepare_prevalidated_source_keys(&preflight_keys)?;

        let mut catalog = Catalog::open(&config.catalog_path())?;
        let cataloged_roots = catalog
            .list_libraries()?
            .into_iter()
            .map(|library| {
                library
                    .canonical_root_key
                    .to_path_buf()
                    .map_err(|_| StartupError::InvalidCatalogPath)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut startup_roots = cataloged_roots;
        startup_roots.push(config.source_root().to_owned());
        // SAFETY: same provenance and restricted lexical use as the preflight
        // capability; the catalog was reopened after local-state preparation.
        let startup_keys =
            unsafe { PrevalidatedSourceKeys::from_validated_identity_keys(startup_roots) }
                .map_err(|_| StartupError::InvalidCatalogPath)?;
        config.validate_prevalidated_source_keys(&startup_keys)?;

        let writer = CacheWriter::new(config.cache_dir())?;
        let report = writer.reconcile_catalog(&mut catalog)?;
        source_startup_hook(SourceStartupTestStage::BeforeFolderConstruction);
        let state = Self {
            catalog: Arc::new(Mutex::new(catalog)),
            cache_root: Arc::new(config.cache_dir().to_owned()),
            folder_root: Some(Arc::new(
                ContainedFolderRoot::from_prevalidated_operational_path(
                    config.source_root().to_owned(),
                ),
            )),
            gallery: None,
            accent_color: config.accent_color().map(ToOwned::to_owned),
            allow_original_downloads: config.allow_original_downloads(),
            original_root: Some(Arc::new(source_startup.clone_original_root()?)),
        };
        source_startup_hook(SourceStartupTestStage::AfterFolderConstruction);
        source_startup_hook(SourceStartupTestStage::BeforeGalleryConstruction);
        let source = source_startup.into_prevalidated_source(
            config.source_root().to_owned(),
            config.source_root().to_owned(),
        )?;
        let gallery = GalleryEngine::open_prevalidated_hosted(
            AppConfig::new(config.data_dir().to_owned(), config.cache_dir().to_owned()),
            source,
        )?;
        source_startup_hook(SourceStartupTestStage::AfterGalleryConstruction);
        Ok((
            Self {
                gallery: Some(Arc::new(gallery)),
                ..state
            },
            report,
        ))
    }

    #[cfg(any(not(unix), debug_assertions))]
    fn open_source_disabled(
        config: &ServerConfig,
    ) -> Result<(Self, CacheReconcileReport), StartupError> {
        let mut preflight_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        preflight_roots.push(config.source_root().to_owned());
        config.validate_source_roots(&preflight_roots)?;
        config.prepare_source_roots(&preflight_roots)?;

        let mut catalog = Catalog::open(&config.catalog_path())?;
        let cataloged_roots = catalog
            .list_libraries()?
            .into_iter()
            .map(|library| {
                library
                    .canonical_root_key
                    .to_path_buf()
                    .map_err(|_| StartupError::InvalidCatalogPath)
            })
            .collect::<Result<Vec<_>, _>>()?;
        config.validate_source_roots(&cataloged_roots)?;
        let writer = CacheWriter::new(config.cache_dir())?;
        let report = writer.reconcile_catalog(&mut catalog)?;
        let mut state = Self::new(catalog, config.cache_dir().to_owned());
        state.accent_color = config.accent_color().map(ToOwned::to_owned);
        state.allow_original_downloads = config.allow_original_downloads();
        Ok((state, report))
    }

    /// Exercises the exact non-Unix source-disabled composer on the current
    /// debug target without making it selectable by Unix release builds.
    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn open_source_disabled_for_test(
        config: &ServerConfig,
    ) -> Result<(Self, CacheReconcileReport), StartupError> {
        Self::open_source_disabled(config)
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn gallery_for_test(&self) -> Option<Arc<GalleryEngine>> {
        self.gallery.clone()
    }
}

pub fn build_router(state: AppState, web_root: StaticWebRoot) -> Router {
    Router::new()
        .route("/api/v1/bootstrap", get(api::bootstrap))
        .route("/api/v1/folders", get(api::folders))
        .route(
            "/api/v1/selections",
            axum::routing::post(api::create_selection),
        )
        .route("/api/v1/selections/{id}", get(api::selection_summary))
        .route("/api/v1/selections/{id}/wall", get(api::wall))
        .route(
            "/api/v1/selections/{id}/assets",
            axum::routing::post(api::resolve_assets),
        )
        .route("/api/v1/selections/{id}/access", get(api::folder_access))
        .route(
            "/api/v1/selections/{id}/interaction",
            axum::routing::post(api::interaction),
        )
        .route("/api/v1/selections/{id}/events", get(api::events))
        .route(
            "/api/v1/selections/{id}/derivatives",
            axum::routing::post(api::request_derivatives),
        )
        .route("/api/v1/derivatives/{id}", get(api::derivative))
        .route("/api/v1/originals/{assetId}", get(api::original))
        .route(
            "/healthz",
            get(|State(state): State<AppState>| async move { health::healthz(state).await }),
        )
        .route("/api/v1", any(api::route_not_found))
        .route("/api/v1/{*path}", any(api::route_not_found))
        .route("/healthz/{*path}", any(api::route_not_found))
        .fallback_service(static_host::router(web_root))
        .with_state(state)
        .layer(CatchPanicLayer::new())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
                let header = |name: &'static str| {
                    request
                        .headers()
                        .get(name)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("-")
                };
                let peer = request
                    .extensions()
                    .get::<ConnectInfo<std::net::SocketAddr>>()
                    .map(|connect| connect.0.to_string())
                    .unwrap_or_else(|| "-".to_owned());
                tracing::info_span!(
                    "http_request",
                    method = %request.method(),
                    peer = %peer,
                    host_untrusted = %header("host"),
                    forwarded_host_untrusted = %header("x-forwarded-host"),
                    forwarded_proto_untrusted = %header("x-forwarded-proto"),
                    forwarded_for_untrusted = %header("x-forwarded-for"),
                )
            }),
        )
}
