#![cfg(unix)]

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_catalog::{Catalog, NewAsset, NewLibrary};
use photo_domain::{LibraryId, MediaKind, RelativePathKey};
use photo_server::{AppState, ServerConfig, build_router};
use std::path::{Path, PathBuf};
use tower::ServiceExt;

const ORIGINAL: &[u8] = b"original image bytes\0\xff";
const SECRET: &[u8] = b"outside source secret";

struct Fixture {
    temp: tempfile::TempDir,
    source: PathBuf,
    catalog: Catalog,
    library: LibraryId,
    app: axum::Router,
}

impl Fixture {
    fn new(enabled: bool) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photos");
        let web = temp.path().join("web");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&web).unwrap();
        let config = ServerConfig::new(
            temp.path().join("data"),
            temp.path().join("cache"),
            None,
            source.clone(),
            web,
        )
        .unwrap()
        .with_allow_original_downloads(enabled);
        let (state, _) = AppState::open(&config).unwrap();
        let catalog = Catalog::open(&config.catalog_path()).unwrap();
        let library = catalog.list_libraries().unwrap()[0].id;
        let app = build_router(state, config.static_web_root());
        Self {
            temp,
            source,
            catalog,
            library,
            app,
        }
    }

    fn asset(&mut self, path: &str) -> String {
        self.asset_in(self.library, path)
    }

    fn asset_in(&mut self, library: LibraryId, path: &str) -> String {
        let asset = NewAsset::minimal(
            library,
            RelativePathKey::from_relative_path(Path::new(path)).unwrap(),
            path,
            MediaKind::Jpeg,
            999,
        );
        self.catalog.upsert_asset(&asset).unwrap();
        asset.id.as_uuid().to_string()
    }

    async fn request(&self, suffix: &str) -> axum::response::Response {
        self.app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/originals/{suffix}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn error(&self, suffix: &str, status: StatusCode, code: &str) {
        let response = self.request(suffix).await;
        assert_eq!(response.status(), status, "{suffix}");
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!text.contains(self.temp.path().to_str().unwrap()), "{text}");
        assert!(
            !text.contains("/"),
            "error must not disclose filesystem paths: {text}"
        );
        assert!(!text.contains("os error"), "{text}");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&text).unwrap()["code"],
            code
        );
    }
}

// Catches a missing capability guard and any lookup/validation before that guard.
#[tokio::test]
async fn disabled_requests_are_identical_for_existing_missing_and_malformed_ids() {
    let mut fixture = Fixture::new(false);
    std::fs::write(fixture.source.join("photo.jpg"), ORIGINAL).unwrap();
    let id = fixture.asset("photo.jpg");
    for id in [
        id.as_str(),
        "00000000-0000-0000-0000-000000000000",
        "not-a-uuid",
        "%2E%2E%2Fsecret",
    ] {
        fixture
            .error(id, StatusCode::FORBIDDEN, "originalDownloadsDisabled")
            .await;
    }
}

// Catches wrong source bytes, catalog-size use, unsafe MIME and inline rendering.
#[tokio::test]
async fn enabled_download_streams_the_original_with_attachment_headers_and_actual_length() {
    let mut fixture = Fixture::new(true);
    std::fs::create_dir(fixture.source.join("trip")).unwrap();
    std::fs::write(fixture.source.join("trip/photo.jpg"), ORIGINAL).unwrap();
    let id = fixture.asset("trip/photo.jpg");
    let response = fixture.request(&id).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()["content-type"],
        "application/octet-stream"
    );
    assert_eq!(response.headers()["content-length"], "22");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    assert_eq!(
        response.headers()["content-disposition"],
        "attachment; filename=\"photo.jpg\"; filename*=UTF-8''photo.jpg"
    );
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        ORIGINAL
    );
}

#[tokio::test]
async fn filename_encodes_unicode_quotes_and_percent_without_header_controls() {
    let mut fixture = Fixture::new(true);
    let name = "été \"100%\"\r\n.jpg";
    std::fs::write(fixture.source.join(name), ORIGINAL).unwrap();
    let id = fixture.asset(name);
    let response = fixture.request(&id).await;
    assert_eq!(response.status(), StatusCode::OK);
    let disposition = response.headers()["content-disposition"].to_str().unwrap();
    assert_eq!(
        disposition,
        "attachment; filename=\"_t_ _100%___.jpg\"; filename*=UTF-8''%C3%A9t%C3%A9%20%22100%25%22__.jpg"
    );
    assert!(!disposition.contains(['\r', '\n']));
}

#[tokio::test]
async fn foreign_and_missing_catalog_assets_cannot_download_even_when_relative_file_exists() {
    let mut fixture = Fixture::new(true);
    std::fs::write(fixture.source.join("photo.jpg"), ORIGINAL).unwrap();
    let foreign = fixture
        .catalog
        .add_library(&NewLibrary::configured(
            "Foreign",
            &fixture.temp.path().join("foreign"),
        ))
        .unwrap();
    let foreign_id = fixture.asset_in(foreign.id, "photo.jpg");
    for id in [foreign_id.as_str(), "00000000-0000-0000-0000-000000000000"] {
        fixture
            .error(id, StatusCode::NOT_FOUND, "originalUnavailable")
            .await;
    }
}

#[tokio::test]
async fn rejects_malformed_encoded_traversal_and_path_query_inputs() {
    let mut fixture = Fixture::new(true);
    let id = fixture.asset("photo.jpg");
    for path in [
        "not-a-uuid",
        "%2E%2E%2Fsecret.jpg",
        "%252E%252E%252Fsecret.jpg",
        "%2Fetc%2Fpasswd",
        "%00",
        "%FF",
    ] {
        fixture
            .error(path, StatusCode::BAD_REQUEST, "invalidRequest")
            .await;
    }
    fixture
        .error(
            &format!("{id}?path=%2Fetc%2Fpasswd"),
            StatusCode::BAD_REQUEST,
            "invalidRequest",
        )
        .await;
}

#[tokio::test]
async fn rejects_catalog_traversal_missing_files_and_non_regular_files() {
    let mut fixture = Fixture::new(true);
    std::fs::write(fixture.temp.path().join("secret.jpg"), SECRET).unwrap();
    std::fs::create_dir(fixture.source.join("directory.jpg")).unwrap();
    use std::os::unix::ffi::OsStrExt;
    let fifo =
        std::ffi::CString::new(fixture.source.join("pipe.jpg").as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
    for path in ["../secret.jpg", "absent.jpg", "directory.jpg", "pipe.jpg"] {
        let id = fixture.asset(path);
        fixture
            .error(&id, StatusCode::NOT_FOUND, "originalUnavailable")
            .await;
    }
}

#[tokio::test]
async fn unavailable_library_does_not_download_an_existing_file() {
    let mut fixture = Fixture::new(true);
    std::fs::write(fixture.source.join("photo.jpg"), ORIGINAL).unwrap();
    let id = fixture.asset("photo.jpg");
    fixture
        .catalog
        .set_library_availability(fixture.library, photo_domain::Availability::RootOffline)
        .unwrap();
    fixture
        .error(&id, StatusCode::NOT_FOUND, "originalUnavailable")
        .await;
}

#[tokio::test]
async fn unavailable_asset_does_not_download_from_an_available_library() {
    let mut fixture = Fixture::new(true);
    std::fs::write(fixture.source.join("photo.jpg"), ORIGINAL).unwrap();
    let id = fixture.asset("photo.jpg");
    fixture.catalog.mark_root_offline(fixture.library).unwrap();
    fixture
        .catalog
        .set_library_availability(fixture.library, photo_domain::Availability::Available)
        .unwrap();
    fixture
        .error(&id, StatusCode::NOT_FOUND, "originalUnavailable")
        .await;
}

#[tokio::test]
async fn unreadable_original_returns_a_path_free_error() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = Fixture::new(true);
    let path = fixture.source.join("private.jpg");
    std::fs::write(&path, ORIGINAL).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0)).unwrap();
    // Root bypasses mode bits; the permission-denied case requires an unprivileged runner.
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let id = fixture.asset("private.jpg");
    fixture
        .error(&id, StatusCode::NOT_FOUND, "originalUnavailable")
        .await;
}

#[tokio::test]
async fn symlinked_files_and_directory_escapes_never_serve_target_bytes() {
    use std::os::unix::fs::symlink;
    let mut fixture = Fixture::new(true);
    let outside = fixture.temp.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::write(outside.join("secret.jpg"), SECRET).unwrap();
    std::fs::write(fixture.source.join("inside.jpg"), ORIGINAL).unwrap();
    symlink(outside.join("secret.jpg"), fixture.source.join("file.jpg")).unwrap();
    symlink(&outside, fixture.source.join("directory")).unwrap();
    symlink(
        fixture.source.join("inside.jpg"),
        fixture.source.join("internal.jpg"),
    )
    .unwrap();
    for path in ["file.jpg", "directory/secret.jpg", "internal.jpg"] {
        let id = fixture.asset(path);
        fixture
            .error(&id, StatusCode::NOT_FOUND, "originalUnavailable")
            .await;
    }
}

#[tokio::test]
async fn source_path_replacement_keeps_serving_the_pinned_source() {
    let mut fixture = Fixture::new(true);
    std::fs::write(fixture.source.join("photo.jpg"), ORIGINAL).unwrap();
    let id = fixture.asset("photo.jpg");
    std::fs::rename(&fixture.source, fixture.temp.path().join("pinned-photos")).unwrap();
    std::fs::create_dir(&fixture.source).unwrap();
    std::fs::write(fixture.source.join("photo.jpg"), SECRET).unwrap();
    let response = fixture.request(&id).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        ORIGINAL
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn replacement_between_startup_validation_and_descriptor_clone_cannot_redirect_downloads() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web = temp.path().join("web");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&web).unwrap();
    std::fs::write(source.join("photo.jpg"), ORIGINAL).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web,
    )
    .unwrap()
    .with_allow_original_downloads(true);
    let (state, _) = AppState::open_with_source_construction_hook(&config, |stage| {
        if stage == photo_server::SourceStartupTestStage::AfterOperationalValidation {
            std::fs::rename(&source, temp.path().join("retained")).unwrap();
            std::fs::create_dir(&source).unwrap();
            std::fs::write(source.join("photo.jpg"), SECRET).unwrap();
        }
    })
    .unwrap();
    let mut catalog = Catalog::open(&config.catalog_path()).unwrap();
    let library = catalog.list_libraries().unwrap()[0].id;
    let asset = NewAsset::minimal(
        library,
        RelativePathKey::from_relative_path(Path::new("photo.jpg")).unwrap(),
        "photo.jpg",
        MediaKind::Jpeg,
        22,
    );
    catalog.upsert_asset(&asset).unwrap();
    let app = build_router(state, config.static_web_root());
    let response = app
        .oneshot(
            Request::get(format!("/api/v1/originals/{}", asset.id.as_uuid()))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .as_ref(),
        ORIGINAL
    );
}
