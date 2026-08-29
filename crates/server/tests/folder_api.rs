use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_catalog::Catalog;
use photo_server::{
    AppState, ConfigError, ContainedFolderRoot, FolderError, ServerConfig, build_router,
};
use tower::ServiceExt;

#[test]
fn config_canonicalizes_source_and_folder_paths_stay_mount_relative() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(source.join("Trips")).unwrap();
    std::fs::create_dir(source.join("Trips").join("Iceland")).unwrap();
    std::fs::write(source.join("photo.jpg"), b"source").unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        temp.path().join("web"),
    )
    .unwrap();
    let root = ContainedFolderRoot::new(source.clone()).unwrap();

    assert_eq!(config.source_root(), source.canonicalize().unwrap());
    assert!(matches!(
        root.list("../outside"),
        Err(FolderError::InvalidPath)
    ));
    assert!(matches!(
        root.list("/absolute"),
        Err(FolderError::InvalidPath)
    ));
    assert!(matches!(
        root.list("photo.jpg"),
        Err(FolderError::NotDirectory)
    ));
    let listing = root.list("Trips").unwrap();
    assert_eq!(listing.path, "Trips");
    assert_eq!(listing.children[0].path, "Trips/Iceland");
    assert!(
        !serde_json::to_string(&listing)
            .unwrap()
            .contains(source.to_str().unwrap())
    );
}

#[cfg(unix)]
#[test]
fn symlink_inside_root_is_listed_and_escape_is_rejected() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let outside = temp.path().join("outside");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::create_dir(source.join("Trips")).unwrap();
    std::fs::create_dir(source.join("Trips").join("Iceland")).unwrap();
    symlink(source.join("Trips"), source.join("Alias")).unwrap();
    symlink(&outside, source.join("Escape")).unwrap();
    let root = ContainedFolderRoot::new(source).unwrap();

    assert!(root.resolve("Alias").is_ok());
    assert!(matches!(
        root.resolve("Escape"),
        Err(FolderError::OutsideRoot)
    ));
    let listing = root.list("").unwrap();
    assert!(listing.children.iter().any(|entry| entry.name == "Alias"));
    assert!(!listing.children.iter().any(|entry| entry.name == "Escape"));
}

#[test]
fn folder_validation_rejects_traversal_nul_and_empty_components() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let root = ContainedFolderRoot::new(source).unwrap();

    for path in [".", "Trips//Processed", "Trips/", "Trips\0Processed"] {
        assert!(
            matches!(root.resolve(path), Err(FolderError::InvalidPath)),
            "{path:?}"
        );
    }
}

#[test]
fn folder_listing_returns_breadcrumbs_sorted_children_and_missing_error() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(source.join("Trips")).unwrap();
    for name in ["zulu", "alpha", "beta"] {
        std::fs::create_dir(source.join("Trips").join(name)).unwrap();
    }
    let root = ContainedFolderRoot::new(source).unwrap();

    let listing = root.list("Trips").unwrap();
    assert_eq!(
        listing.breadcrumbs,
        vec![photo_server::FolderBreadcrumb {
            name: "Trips".to_owned(),
            path: "Trips".to_owned(),
        }]
    );
    assert_eq!(
        listing
            .children
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["alpha", "beta", "zulu"]
    );
    assert!(matches!(
        root.list("Trips/Missing"),
        Err(FolderError::Unavailable)
    ));
}

#[test]
fn folder_listing_is_case_insensitive_before_bytewise_ordering() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    for name in ["apple", "Beta"] {
        std::fs::create_dir(source.join(name)).unwrap();
    }
    let root = ContainedFolderRoot::new(source).unwrap();

    let listing = root.list("").unwrap();
    assert_eq!(
        listing
            .children
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["apple", "Beta"]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn folder_listing_uses_original_name_as_case_insensitive_tie_breaker() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    for name in ["alpha", "ALPHA"] {
        std::fs::create_dir(source.join(name)).unwrap();
    }
    let root = ContainedFolderRoot::new(source).unwrap();

    let listing = root.list("").unwrap();
    assert_eq!(
        listing
            .children
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>(),
        vec!["ALPHA", "alpha"]
    );
}

#[cfg(unix)]
#[test]
fn mode_bits_make_unreadable_directories_fail_resolution() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let unreadable = source.join("Private");
    std::fs::create_dir(&unreadable).unwrap();
    // Read permission without traversal permission is not enough to enumerate
    // a directory. The real read_dir result makes this meaningful even if a
    // privileged runner bypasses mode bits.
    std::fs::set_permissions(&unreadable, std::fs::Permissions::from_mode(0o400)).unwrap();
    let root = ContainedFolderRoot::new(source).unwrap();

    let resolved = root.resolve("Private");
    if std::fs::read_dir(&unreadable).is_ok() {
        assert!(resolved.is_ok());
    } else {
        assert!(matches!(resolved, Err(FolderError::Unreadable)));
    }
}

#[tokio::test]
async fn folder_api_returns_bootstrap_and_rejects_oversized_or_repeated_paths() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(source.join("Trips")).unwrap();
    std::fs::create_dir(source.join("Trips").join("Iceland")).unwrap();
    let source_file = source.join("photo.jpg");
    std::fs::write(&source_file, b"source-bytes").unwrap();
    let source_before = std::fs::read(&source_file).unwrap();
    let source_metadata_before = std::fs::metadata(&source_file).unwrap();
    let state = AppState::new_with_source_root(
        Catalog::open_in_memory().unwrap(),
        temp.path().join("cache"),
        source,
    )
    .unwrap();
    let app = build_router(state);

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/bootstrap")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["capabilities"]["folderBrowser"], true);
    assert_eq!(value["capabilities"]["video"], false);
    assert_eq!(value["sourceAvailable"], true);
    assert!(!value.to_string().contains("photos"));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/v1/folders?path=Trips")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["path"], "Trips");
    assert_eq!(value["children"][0]["name"], "Iceland");
    assert_eq!(value["children"][0]["path"], "Trips/Iceland");
    assert!(!value.to_string().contains("photos"));

    let oversized = format!("/api/v1/folders?path={}", "x".repeat(4097));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(oversized)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["code"], "invalidFolderPath");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/folders?path=Trips&path=Other")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["code"], "invalidFolderPath");

    let source_metadata_after = std::fs::metadata(&source_file).unwrap();
    assert_eq!(std::fs::read(&source_file).unwrap(), source_before);
    assert_eq!(source_metadata_after.len(), source_metadata_before.len());
    assert_eq!(
        source_metadata_after.modified().unwrap(),
        source_metadata_before.modified().unwrap()
    );
}

#[tokio::test]
async fn invalid_folder_queries_are_rejected_before_filesystem_access() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let state = AppState::new_with_source_root(
        Catalog::open_in_memory().unwrap(),
        temp.path().join("cache"),
        source.clone(),
    )
    .unwrap();
    std::fs::remove_dir(&source).unwrap();
    let app = build_router(state);

    let oversized = format!("/api/v1/folders?path={}", "x".repeat(4097));
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(oversized)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["code"], "invalidFolderPath");

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/folders?path=Trips&path=Other")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["code"], "invalidFolderPath");
}

#[tokio::test]
async fn mounted_root_missing_file_and_unreadable_fail_as_source_unavailable() {
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let state = AppState::new_with_source_root(
        Catalog::open_in_memory().unwrap(),
        temp.path().join("cache-missing"),
        source.clone(),
    )
    .unwrap();
    std::fs::remove_dir(&source).unwrap();
    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/folders")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["code"], "sourceUnavailable");

    let source = temp.path().join("file-source");
    std::fs::create_dir(&source).unwrap();
    let state = AppState::new_with_source_root(
        Catalog::open_in_memory().unwrap(),
        temp.path().join("cache-file"),
        source.clone(),
    )
    .unwrap();
    std::fs::remove_dir(&source).unwrap();
    std::fs::write(&source, b"not a directory").unwrap();
    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/folders?path=")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["code"], "sourceUnavailable");

    let source = temp.path().join("unreadable-source");
    std::fs::create_dir(&source).unwrap();
    let state = AppState::new_with_source_root(
        Catalog::open_in_memory().unwrap(),
        temp.path().join("cache-unreadable"),
        source.clone(),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Privileged runners can still enumerate this directory. In that
        // environment there is no unreadable case to assert, so restore and
        // skip only after the real operation demonstrably succeeds.
        if std::fs::read_dir(&source).is_ok() {
            std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
            return;
        }
    }
    #[cfg(not(unix))]
    return;
    let app = build_router(state);
    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/folders?path=")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(value["code"], "sourceUnavailable");
}

#[test]
fn config_rejects_a_source_that_is_missing_or_overlaps_local_state() {
    let temp = tempfile::tempdir().unwrap();
    let missing = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        temp.path().join("missing"),
        temp.path().join("web"),
    );
    assert!(matches!(missing, Err(ConfigError::Io(_))));

    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let overlap = ServerConfig::new(
        source.join("data"),
        temp.path().join("cache"),
        None,
        source,
        temp.path().join("web"),
    );
    assert!(matches!(overlap, Err(ConfigError::InsideSourceRoot)));
}
