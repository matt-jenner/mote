use std::path::Path;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_catalog::{Catalog, NewAsset, NewDerivative, NewFolderGroup, NewLibrary};
use photo_domain::{Availability, DerivativeId, FolderGroupId, MediaKind, RelativePathKey};
use photo_server::{AppState, ConfigError, ServerConfig, StartupError, build_router};
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

#[test]
fn server_config_translates_shared_local_state_overlap_errors() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        source.join("app-data"),
        temp.path().join("cache"),
        None,
        vec![],
    )
    .unwrap();

    assert!(matches!(
        config.validate_source_roots(&[source]),
        Err(ConfigError::InsideSourceRoot)
    ));
}

#[cfg(unix)]
#[test]
fn config_rejects_a_symlink_alias_into_a_source_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let alias = temp.path().join("photo-alias");
    std::fs::create_dir(&source).unwrap();
    symlink(&source, &alias).unwrap();
    let config = ServerConfig::new(
        alias.join("app-data"),
        temp.path().join("cache"),
        None,
        vec![source.clone()],
    )
    .unwrap();

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

#[test]
fn application_startup_repairs_cache_before_serving() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&cache).unwrap();
    let config = ServerConfig::new(data, cache.clone(), None, vec![]).unwrap();
    let (group, missing_id) = catalog_with_missing_derivative(&config, &source);
    std::fs::write(cache.join("orphan.partial-test"), b"partial").unwrap();

    let (state, report) = AppState::open(&config).unwrap();

    assert_eq!(report.partial_files_removed, 1);
    assert_eq!(report.missing_rows_removed, 1);
    drop(state);
    let catalog = Catalog::open(&config.catalog_path()).unwrap();
    assert_eq!(catalog.derivative_count(group, false).unwrap(), 0);
    assert!(
        catalog
            .all_derivatives()
            .unwrap()
            .iter()
            .all(|record| record.id != missing_id)
    );
}

#[test]
fn application_preflights_cataloged_roots_before_creating_cache_state() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let data_inside_source = source.join("viewer-data");
    let cache = temp.path().join("cache-not-created");
    std::fs::create_dir_all(&data_inside_source).unwrap();
    let config = ServerConfig::new(data_inside_source, cache.clone(), None, vec![]).unwrap();
    let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
    catalog
        .add_library(&NewLibrary::configured("Photos", &source))
        .unwrap();
    drop(catalog);

    let result = AppState::open(&config);

    assert!(matches!(
        result,
        Err(StartupError::Config(ConfigError::InsideSourceRoot))
    ));
    assert!(!cache.exists());
}

#[cfg(unix)]
#[test]
fn application_rejects_a_catalog_symlink_into_a_source_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let source_catalog = source.join("catalog.sqlite");
    let data = temp.path().join("data");
    let cache = temp.path().join("cache-not-created");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&data).unwrap();
    let mut catalog = Catalog::open(&source_catalog).unwrap();
    catalog
        .add_library(&NewLibrary::configured("Photos", &source))
        .unwrap();
    drop(catalog);
    symlink(&source_catalog, data.join("catalog.sqlite")).unwrap();
    let config = ServerConfig::new(data, cache.clone(), None, vec![]).unwrap();

    let result = AppState::open(&config);

    assert!(matches!(
        result,
        Err(StartupError::Config(ConfigError::InsideSourceRoot))
    ));
    assert!(!cache.exists());
}

#[test]
fn application_rejects_a_source_nested_inside_the_cache_root() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let source = cache.join("photos");
    let source_file = source.join("original.partial-camera.jpg");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(&source_file, b"source").unwrap();
    let config = ServerConfig::new(data, cache, None, vec![]).unwrap();
    let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
    catalog
        .add_library(&NewLibrary::configured("Photos", &source))
        .unwrap();
    drop(catalog);

    let result = AppState::open(&config);

    assert!(matches!(
        result,
        Err(StartupError::Config(ConfigError::InsideSourceRoot))
    ));
    assert_eq!(std::fs::read(source_file).unwrap(), b"source");
}

fn add_library(catalog: &mut Catalog, root: &str, availability: Availability) {
    let library = NewLibrary::configured(root, Path::new(root));
    catalog.add_library(&library).unwrap();
    catalog
        .set_library_availability(library.id, availability)
        .unwrap();
}

fn catalog_with_missing_derivative(
    config: &ServerConfig,
    source: &Path,
) -> (FolderGroupId, DerivativeId) {
    let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
    let library = NewLibrary::configured("Photos", source);
    catalog.add_library(&library).unwrap();
    let relative = RelativePathKey::from_relative_path(Path::new("collection/image.jpg")).unwrap();
    let asset = NewAsset::minimal(
        library.id,
        relative,
        "collection/image.jpg",
        MediaKind::Jpeg,
        100,
    );
    catalog.upsert_asset(&asset).unwrap();
    let proposed_group = FolderGroupId::new();
    let group = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: proposed_group,
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(Path::new("collection")).unwrap(),
            display_path: "collection".to_owned(),
            last_viewed_at: Some(1),
        })
        .unwrap();
    let derivative = DerivativeId::new();
    catalog
        .insert_derivative(&NewDerivative {
            id: derivative,
            asset_id: asset.id,
            folder_group_id: group,
            kind: "screen_preview".to_owned(),
            cache_key: "missing-preview".to_owned(),
            relative_cache_path: "aa/bb/missing.preview".into(),
            size_bytes: 10,
            durable: false,
            created_at: 1,
        })
        .unwrap();
    (group, derivative)
}
