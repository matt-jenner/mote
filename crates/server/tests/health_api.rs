use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_catalog::{Catalog, NewAsset, NewDerivative, NewFolderGroup, NewLibrary};
use photo_domain::{Availability, DerivativeId, FolderGroupId, MediaKind, RelativePathKey};
#[cfg(unix)]
use photo_server::SourceStartupTestStage;
use photo_server::{AppState, ConfigError, ServerConfig, StaticWebRoot, build_router};
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
    let app = build_router(
        AppState::new(catalog, cache.path().to_owned()),
        StaticWebRoot::open(web_root(&cache)).unwrap(),
    );

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
    let app = build_router(
        AppState::new(catalog, not_a_directory),
        StaticWebRoot::open(web_root(&temp)).unwrap(),
    );

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

#[cfg(unix)]
#[tokio::test]
async fn root_folder_refresh_drives_path_free_source_health() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let nested = source.join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let cache = temp.path().join("cache");
    std::fs::create_dir(&cache).unwrap();
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = NewLibrary::configured("Photos", &source.canonicalize().unwrap());
    catalog.add_library(&library).unwrap();
    let app = build_router(
        AppState::new_with_source_root(catalog, cache, source.clone()).unwrap(),
        StaticWebRoot::open(web_root(&temp)).unwrap(),
    );

    let (status, healthy) = request_json(&app, "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(healthy["status"], "healthy");
    assert_eq!(healthy["sources"]["available"], 1);
    assert_eq!(healthy["sources"]["unavailable"], 0);

    let nested_permissions = std::fs::metadata(&nested).unwrap().permissions();
    let mut unreadable_nested = nested_permissions.clone();
    unreadable_nested.set_mode(0o000);
    std::fs::set_permissions(&nested, unreadable_nested).unwrap();
    let nested_response = request_json(&app, "/api/v1/folders?path=nested").await;
    std::fs::set_permissions(&nested, nested_permissions).unwrap();
    assert_eq!(nested_response.0, StatusCode::FORBIDDEN);
    assert_eq!(request_json(&app, "/healthz").await.1["status"], "healthy");

    let source_permissions = std::fs::metadata(&source).unwrap().permissions();
    let mut unreadable_source = source_permissions.clone();
    unreadable_source.set_mode(0o000);
    std::fs::set_permissions(&source, unreadable_source).unwrap();
    let root_response = request_json(&app, "/api/v1/folders?path=").await;
    std::fs::set_permissions(&source, source_permissions).unwrap();
    assert_eq!(root_response.0, StatusCode::SERVICE_UNAVAILABLE);

    let (status, degraded) = request_json(&app, "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(degraded["status"], "degraded");
    assert_eq!(degraded["sources"]["available"], 0);
    assert_eq!(degraded["sources"]["unavailable"], 1);
    assert!(
        !degraded
            .to_string()
            .contains(source.to_string_lossy().as_ref())
    );

    assert_eq!(
        request_json(&app, "/api/v1/folders?path=").await.0,
        StatusCode::OK
    );
    let (_, recovered) = request_json(&app, "/healthz").await;
    assert_eq!(recovered["status"], "healthy");
    assert_eq!(recovered["sources"]["available"], 1);
    assert_eq!(recovered["sources"]["unavailable"], 0);
    assert!(
        !recovered
            .to_string()
            .contains(source.to_string_lossy().as_ref())
    );
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
        source.clone(),
        web_root(&temp),
    );

    assert!(matches!(config, Err(ConfigError::InsideSourceRoot)));
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
        source.clone(),
        web_root(&temp),
    );

    assert!(matches!(config, Err(ConfigError::InsideSourceRoot)));
}

#[test]
fn server_config_requires_an_existing_web_directory() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web = temp.path().join("web");
    let web_file = temp.path().join("web-file");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&web).unwrap();
    std::fs::write(&web_file, b"not a directory").unwrap();

    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web.clone(),
    )
    .unwrap();
    assert_eq!(config.web_root(), web.canonicalize().unwrap());

    for invalid_web_root in [temp.path().join("missing-web"), web_file] {
        let result = ServerConfig::new(
            temp.path().join("other-data"),
            temp.path().join("other-cache"),
            None,
            source.clone(),
            invalid_web_root,
        );
        assert!(result.is_err());
    }
}

#[test]
fn server_config_rejects_web_root_overlap_with_source_or_private_state() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web_inside_source = source.join("web");
    std::fs::create_dir_all(&web_inside_source).unwrap();
    let source_overlap = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        web_inside_source,
    );
    assert!(source_overlap.is_err());

    let source = temp.path().join("other-photos");
    let web = temp.path().join("public-web");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&web).unwrap();
    let private_state_overlap = ServerConfig::new(
        web.join("data"),
        temp.path().join("other-cache"),
        None,
        source,
        web,
    );
    assert!(private_state_overlap.is_err());
}

#[cfg(unix)]
#[test]
fn missing_local_children_use_their_pinned_existing_parent() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web = temp.path().join("web");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&web).unwrap();

    let beneath_source = ServerConfig::new(
        source.join("missing-data"),
        temp.path().join("cache-a"),
        None,
        source.clone(),
        web.clone(),
    );
    assert!(matches!(beneath_source, Err(ConfigError::InsideSourceRoot)));

    let beneath_web = ServerConfig::new(
        temp.path().join("data-b"),
        web.join("missing-cache"),
        None,
        source.clone(),
        web.clone(),
    );
    assert!(matches!(beneath_web, Err(ConfigError::InsideSourceRoot)));

    let disjoint = ServerConfig::new(
        temp.path().join("disjoint-data"),
        temp.path().join("disjoint-cache"),
        None,
        source,
        web,
    );
    assert!(disjoint.is_ok());
}

#[cfg(unix)]
#[test]
fn application_rejects_a_source_swap_at_the_operational_startup_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let replacement = temp.path().join("replacement");
    let moved_source = temp.path().join("validated-source");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(source.join("safe.jpg"), b"SAFE SOURCE").unwrap();
    std::fs::write(replacement.join("sentinel.jpg"), b"SOURCE SENTINEL").unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web_root(&temp),
    )
    .unwrap();

    let result = AppState::open_with_source_startup_hook(&config, || {
        std::fs::rename(&source, &moved_source).unwrap();
        std::fs::rename(&replacement, &source).unwrap();
    });

    assert!(result.is_err());
    assert_eq!(
        std::fs::read(source.join("sentinel.jpg")).unwrap(),
        b"SOURCE SENTINEL"
    );
    let catalog = Catalog::open(&config.catalog_path()).unwrap();
    assert!(catalog.list_libraries().unwrap().is_empty());
}

#[cfg(unix)]
#[test]
fn unchanged_source_identity_crosses_the_operational_startup_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        web_root(&temp),
    )
    .unwrap();

    let result = AppState::open_with_source_startup_hook(&config, || {});

    assert!(result.is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn transient_source_swap_cannot_retarget_the_folder_root() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let moved_source = temp.path().join("validated-source");
    let replacement = temp.path().join("replacement");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(source.join("safe-album")).unwrap();
    std::fs::create_dir(&replacement).unwrap();
    std::fs::create_dir(replacement.join("source-sentinel-album")).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web_root(&temp),
    )
    .unwrap();

    let (state, _) = AppState::open_with_source_construction_hook(&config, |stage| match stage {
        SourceStartupTestStage::BeforeFolderConstruction => {
            std::fs::rename(&source, &moved_source).unwrap();
            symlink(&replacement, &source).unwrap();
        }
        SourceStartupTestStage::AfterFolderConstruction => {
            std::fs::remove_file(&source).unwrap();
            std::fs::rename(&moved_source, &source).unwrap();
        }
        _ => {}
    })
    .unwrap();
    let app = build_router(state, config.static_web_root());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/folders")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();

    assert_eq!(value["children"][0]["name"], "safe-album");
    assert!(!value.to_string().contains("source-sentinel-album"));
}

#[cfg(unix)]
#[test]
fn transient_source_swap_cannot_change_the_configured_library_key() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let moved_source = temp.path().join("validated-source");
    let replacement = temp.path().join("replacement");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&replacement).unwrap();
    std::fs::write(replacement.join("source-sentinel.jpg"), b"SOURCE SENTINEL").unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web_root(&temp),
    )
    .unwrap();

    let result = AppState::open_with_source_construction_hook(&config, |stage| match stage {
        SourceStartupTestStage::BeforeGalleryConstruction => {
            std::fs::rename(&source, &moved_source).unwrap();
            symlink(&replacement, &source).unwrap();
        }
        SourceStartupTestStage::AfterGalleryConstruction => {
            std::fs::remove_file(&source).unwrap();
            std::fs::rename(&moved_source, &source).unwrap();
        }
        _ => {}
    });

    assert!(result.is_ok());
    assert_eq!(
        Catalog::read_library_root_paths(&config.catalog_path()).unwrap(),
        vec![source.canonicalize().unwrap()]
    );
    assert_eq!(
        std::fs::read(replacement.join("source-sentinel.jpg")).unwrap(),
        b"SOURCE SENTINEL"
    );
}

#[cfg(unix)]
#[test]
fn prevalidated_startup_does_not_reopen_the_source_path_during_construction() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let offline_source = temp.path().join("photos-offline");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("safe.jpg"), b"SAFE SOURCE").unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web_root(&temp),
    )
    .unwrap();

    let result = AppState::open_with_source_construction_hook(&config, |stage| match stage {
        SourceStartupTestStage::AfterOperationalValidation => {
            std::fs::rename(&source, &offline_source).unwrap();
        }
        SourceStartupTestStage::AfterGalleryConstruction => {
            std::fs::rename(&offline_source, &source).unwrap();
        }
        _ => {}
    });

    assert!(result.is_ok());
    assert_eq!(
        Catalog::read_library_root_paths(&config.catalog_path()).unwrap(),
        vec![source.canonicalize().unwrap()]
    );
}

#[cfg(unix)]
#[test]
fn source_startup_validation_is_one_shot_and_released_after_success() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web_root(&temp),
    )
    .unwrap();
    let clone = config.clone();
    let release = config.source_startup_release_probe().unwrap();

    let first = AppState::open(&config);
    let second = AppState::open(&clone);

    assert!(first.is_ok());
    assert!(release.upgrade().is_none());
    assert!(matches!(
        &second,
        Err(photo_server::StartupError::Config(
            ConfigError::SourceStartupUnavailable
        ))
    ));
    let error = match second {
        Err(error) => error.to_string(),
        Ok(_) => panic!("second startup unexpectedly succeeded"),
    };
    assert!(!error.contains(source.to_string_lossy().as_ref()));
}

#[cfg(unix)]
#[test]
fn source_startup_descriptor_is_released_on_error() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        web_root(&temp),
    )
    .unwrap();
    std::fs::create_dir_all(config.catalog_path().parent().unwrap()).unwrap();
    std::fs::write(config.catalog_path(), b"not a sqlite catalog").unwrap();
    let release = config.source_startup_release_probe().unwrap();

    let result = AppState::open(&config);

    assert!(result.is_err());
    assert!(release.upgrade().is_none());
    assert!(matches!(
        AppState::open(&config),
        Err(photo_server::StartupError::Config(
            ConfigError::SourceStartupUnavailable
        ))
    ));
}

#[cfg(unix)]
#[test]
fn source_startup_descriptor_is_released_during_unwind() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        web_root(&temp),
    )
    .unwrap();
    let release = config.source_startup_release_probe().unwrap();

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = AppState::open_with_source_construction_hook(&config, |stage| {
            if stage == SourceStartupTestStage::AfterOperationalValidation {
                panic!("deterministic startup panic");
            }
        });
    }));

    assert!(result.is_err());
    assert!(release.upgrade().is_none());
    assert!(matches!(
        AppState::open(&config),
        Err(photo_server::StartupError::Config(
            ConfigError::SourceStartupUnavailable
        ))
    ));
}

#[cfg(unix)]
#[test]
fn normalized_prevalidated_library_key_rejects_catalog_overlap() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web_root(&temp),
    )
    .unwrap();
    let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
    catalog
        .add_library(&NewLibrary::configured(
            "Nested",
            &source.join("child").join("..").join("child"),
        ))
        .unwrap();
    drop(catalog);
    let release = config.source_startup_release_probe().unwrap();

    let result = AppState::open(&config);

    assert!(matches!(
        result,
        Err(photo_server::StartupError::Gallery(
            photo_app_service::AppServiceError::OpenRecent(
                photo_core::AddLibraryError::Overlaps { .. }
            )
        ))
    ));
    assert!(release.upgrade().is_none());
    assert!(matches!(
        AppState::open(&config),
        Err(photo_server::StartupError::Config(
            ConfigError::SourceStartupUnavailable
        ))
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
        alias,
        web_root(&temp),
    );

    assert!(matches!(config, Err(ConfigError::InsideSourceRoot)));
    assert!(!source.join("app-data").exists());
}

#[cfg(unix)]
#[test]
fn config_creates_private_local_directories() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        Some("127.0.0.1:0"),
        source,
        web_root(&temp),
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
    let config =
        ServerConfig::new(data, cache.clone(), None, source.clone(), web_root(&temp)).unwrap();
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
    let configured_source = temp.path().join("configured-photos");
    let data_inside_source = source.join("viewer-data");
    let cache = temp.path().join("cache-not-created");
    std::fs::create_dir_all(&data_inside_source).unwrap();
    std::fs::create_dir(&configured_source).unwrap();
    let config = ServerConfig::new(
        data_inside_source,
        cache.clone(),
        None,
        configured_source,
        web_root(&temp),
    )
    .unwrap();
    let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
    catalog
        .add_library(&NewLibrary::configured("Photos", &source))
        .unwrap();
    drop(catalog);

    let result = AppState::open(&config);

    assert!(matches!(
        result,
        Err(photo_server::StartupError::Config(
            ConfigError::InsideSourceRoot
        ))
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
    let configured_source = temp.path().join("configured-photos");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&data).unwrap();
    std::fs::create_dir(&configured_source).unwrap();
    let mut catalog = Catalog::open(&source_catalog).unwrap();
    catalog
        .add_library(&NewLibrary::configured("Photos", &source))
        .unwrap();
    drop(catalog);
    symlink(&source_catalog, data.join("catalog.sqlite")).unwrap();
    let config = ServerConfig::new(
        data,
        cache.clone(),
        None,
        configured_source,
        web_root(&temp),
    )
    .unwrap();

    let result = AppState::open(&config);

    assert!(matches!(
        result,
        Err(photo_server::StartupError::Config(
            ConfigError::InsideSourceRoot
        ))
    ));
    assert!(!cache.exists());
}

#[test]
fn application_rejects_a_source_nested_inside_the_cache_root() {
    let temp = tempfile::tempdir().unwrap();
    let data = temp.path().join("data");
    let cache = temp.path().join("cache");
    let configured_source = temp.path().join("configured-photos");
    let source = cache.join("photos");
    let source_file = source.join("original.partial-camera.jpg");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir(&configured_source).unwrap();
    std::fs::write(&source_file, b"source").unwrap();
    let config = ServerConfig::new(data, cache, None, configured_source, web_root(&temp)).unwrap();
    let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
    catalog
        .add_library(&NewLibrary::configured("Photos", &source))
        .unwrap();
    drop(catalog);

    let result = AppState::open(&config);

    assert!(matches!(
        result,
        Err(photo_server::StartupError::Config(
            ConfigError::InsideSourceRoot
        ))
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

#[cfg(unix)]
async fn request_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (status, value)
}

fn web_root(temp: &tempfile::TempDir) -> PathBuf {
    let web = temp.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    web
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
