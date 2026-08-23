use std::path::PathBuf;

use photo_catalog::Catalog;
use photo_server::{AppState, ServerConfig, build_router};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("photo_server=info")),
        )
        .init();

    let config = ServerConfig::from_env(Vec::new())?;
    config.prepare()?;
    let catalog = Catalog::open(&config.catalog_path())?;
    let source_roots = catalog
        .list_libraries()?
        .into_iter()
        .map(|library| PathBuf::from(library.display_path))
        .collect::<Vec<_>>();
    config.validate_source_roots(&source_roots)?;

    let listener = tokio::net::TcpListener::bind(config.bind()).await?;
    tracing::info!(bind = %config.bind(), "photo catalog health service started");
    axum::serve(
        listener,
        build_router(AppState::new(catalog, config.cache_dir().to_owned())),
    )
    .await?;
    Ok(())
}
