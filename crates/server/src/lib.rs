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
use photo_app_service::{AppConfig, AppServiceError, GalleryEngine};
use photo_cache::{CacheReconcileReport, CacheWriter};
use photo_catalog::Catalog;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::TraceLayer;

pub use config::{ConfigError, ServerConfig};
pub use folders::{ContainedFolderRoot, FolderBreadcrumb, FolderEntry, FolderError, FolderListing};
pub use health::{
    ComponentHealth, ComponentStatus, HealthReport, HealthStatus, SourceHealthCounts,
};

#[derive(Clone)]
pub struct AppState {
    pub(crate) catalog: Arc<Mutex<Catalog>>,
    pub(crate) cache_root: Arc<PathBuf>,
    pub(crate) folder_root: Option<Arc<ContainedFolderRoot>>,
    pub(crate) gallery: Option<Arc<GalleryEngine>>,
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
    pub fn new(catalog: Catalog, cache_root: PathBuf) -> Self {
        Self {
            catalog: Arc::new(Mutex::new(catalog)),
            cache_root: Arc::new(cache_root),
            folder_root: None,
            gallery: None,
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
        })
    }

    pub fn open(config: &ServerConfig) -> Result<(Self, CacheReconcileReport), StartupError> {
        let preflight_roots = Catalog::read_library_root_paths(&config.catalog_path())?;
        config.validate_source_roots(&preflight_roots)?;
        config.prepare()?;

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
        let state = Self::new_with_source_root(
            catalog,
            config.cache_dir().to_owned(),
            config.source_root().to_owned(),
        )?;
        let gallery = GalleryEngine::open(
            AppConfig::new(config.data_dir().to_owned(), config.cache_dir().to_owned()),
            config.source_root().to_owned(),
        )?;
        Ok((
            Self {
                gallery: Some(Arc::new(gallery)),
                ..state
            },
            report,
        ))
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn gallery_for_test(&self) -> Option<Arc<GalleryEngine>> {
        self.gallery.clone()
    }
}

pub fn build_router(state: AppState, web_root: PathBuf) -> Router {
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
            "/api/v1/selections/{id}/interaction",
            axum::routing::post(api::interaction),
        )
        .route("/api/v1/selections/{id}/events", get(api::events))
        .route(
            "/api/v1/selections/{id}/derivatives",
            axum::routing::post(api::request_derivatives),
        )
        .route("/api/v1/derivatives/{id}", get(api::derivative))
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
