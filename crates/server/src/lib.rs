mod config;
mod health;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::State;
use axum::routing::get;
use photo_catalog::Catalog;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::TraceLayer;

pub use config::{ConfigError, ServerConfig};
pub use health::{
    ComponentHealth, ComponentStatus, HealthReport, HealthStatus, SourceHealthCounts,
};

#[derive(Clone)]
pub struct AppState {
    pub(crate) catalog: Arc<Mutex<Catalog>>,
    pub(crate) cache_root: Arc<PathBuf>,
}

impl AppState {
    pub fn new(catalog: Catalog, cache_root: PathBuf) -> Self {
        Self {
            catalog: Arc::new(Mutex::new(catalog)),
            cache_root: Arc::new(cache_root),
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/healthz",
            get(|State(state): State<AppState>| async move { health::healthz(state).await }),
        )
        .with_state(state)
        .layer(CatchPanicLayer::new())
        .layer(
            TraceLayer::new_for_http().make_span_with(|request: &axum::http::Request<_>| {
                tracing::info_span!("http_request", method = %request.method())
            }),
        )
}
