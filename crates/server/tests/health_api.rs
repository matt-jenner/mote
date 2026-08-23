use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_catalog::{Catalog, NewLibrary};
use photo_domain::Availability;
use photo_server::{AppState, ConfigError, ServerConfig, build_router};
use tower::ServiceExt;

#[tokio::test]
async fn health_reports_database_cache_and_source_counts_without_paths() {
    let cache = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open_in_memory().unwrap();
    add_library(&mut catalog, "/Volumes/family-a", Availability::Available);
    add_library(&mut catalog, "/Volumes/family-b", Availability::Available);
    add_library(
        &mut catalog,
        "/Volumes/offline-family",
        Availability::RootOffline,
    );
    let app = build_router(AppState::new(catalog, cache.path().to_owned()));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["status"], "degraded");
    assert_eq!(value["database"]["status"], "healthy");
    assert_eq!(value["cache"]["status"], "healthy");
    assert_eq!(value["sources"]["available"], 2);
    assert_eq!(value["sources"]["unavailable"], 1);
    assert_eq!(value["active_warnings"], 0);
    assert!(!value.to_string().contains("/Volumes"));
    assert!(!value.to_string().contains("family"));
}

#[tokio::test]
async fn unwritable_cache_is_unhealthy_and_returns_service_unavailable() {
    let temp = tempfile::tempdir().unwrap();
    let not_a_directory = temp.path().join("cache-file");
    std::fs::write(&not_a_directory, b"not a directory").unwrap();
    let catalog = Catalog::open_in_memory().unwrap();
    let app = build_router(AppState::new(catalog, not_a_directory));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["status"], "unhealthy");
    assert_eq!(value["cache"]["status"], "unhealthy");
}

#[test]
fn config_defaults_to_loopback_and_rejects_local_state_inside_a_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        source.join("app-data"),
        temp.path().join("cache"),
        None,
        vec![source.clone()],
    )
    .unwrap();

    assert_eq!(config.bind().to_string(), "127.0.0.1:8080");
    assert!(matches!(
        config.prepare(),
        Err(ConfigError::InsideSourceRoot)
    ));
    assert!(!source.join("app-data").exists());
}

#[cfg(unix)]
#[test]
fn config_creates_private_local_directories() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        Some("127.0.0.1:0"),
        vec![],
    )
    .unwrap();
    config.prepare().unwrap();

    assert_eq!(
        std::fs::metadata(config.data_dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(config.cache_dir())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

fn add_library(catalog: &mut Catalog, root: &str, availability: Availability) {
    let library = NewLibrary::configured(root, Path::new(root));
    catalog.add_library(&library).unwrap();
    catalog
        .set_library_availability(library.id, availability)
        .unwrap();
}
