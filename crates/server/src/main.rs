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

    let config = ServerConfig::from_env()?;
    let (state, repair) = AppState::open(&config)?;
    if repair.partial_files_removed > 0 || repair.missing_rows_removed > 0 {
        tracing::info!(
            partial_files_removed = repair.partial_files_removed,
            missing_rows_removed = repair.missing_rows_removed,
            "repaired derivative cache state"
        );
    }
    if !config.bind().ip().is_loopback() {
        tracing::warn!(
            warning_code = "broad_bind",
            bind = %config.bind(),
            "health service is exposed beyond loopback"
        );
    }

    let listener = tokio::net::TcpListener::bind(config.bind()).await?;
    tracing::info!(bind = %config.bind(), "photo catalog health service started");
    axum::serve(listener, build_router(state)).await?;
    Ok(())
}
