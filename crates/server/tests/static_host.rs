use axum::body::Body;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_ENCODING, CONTENT_RANGE, CONTENT_SECURITY_POLICY, CONTENT_TYPE, ETAG,
    IF_NONE_MATCH, LOCATION, RANGE, REFERRER_POLICY, VARY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use photo_catalog::Catalog;
use photo_server::{AppState, ServerConfig, StaticWebRoot, build_router};
use tempfile::TempDir;
use tower::ServiceExt;

const INDEX: &str = r#"<!doctype html><title>Photo Viewer</title><script type="module" src="/assets/app-immutable.js"></script>"#;
const JAVASCRIPT: &str = "globalThis.photoViewer = true;";
const BROTLI_JAVASCRIPT: &[u8] = b"precompressed-javascript";
const BROTLI_INDEX: &[u8] = b"precompressed-index";
const GZIP_JAVASCRIPT: &[u8] = b"gzip-precompressed-javascript";
const API_SHADOW: &str = "PHYSICAL API SHADOW";
const HEALTH_SHADOW: &str = "PHYSICAL HEALTH SHADOW";
const APIARY_ASSET: &str = "BOUNDARY ASSET";
const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

fn app() -> (TempDir, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let web = temp.path().join("web");
    let assets = web.join("assets");
    let cache = temp.path().join("cache");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::create_dir(&cache).unwrap();
    std::fs::write(web.join("index.html"), INDEX).unwrap();
    std::fs::write(web.join("index.html.br"), BROTLI_INDEX).unwrap();
    std::fs::write(assets.join("app-immutable.js"), JAVASCRIPT).unwrap();
    std::fs::write(assets.join("app-immutable.js.br"), BROTLI_JAVASCRIPT).unwrap();
    std::fs::write(assets.join("app-immutable.js.gz"), GZIP_JAVASCRIPT).unwrap();
    std::fs::create_dir_all(web.join("api/v1")).unwrap();
    std::fs::write(web.join("api/v1/unknown"), API_SHADOW).unwrap();
    std::fs::create_dir(web.join("healthz")).unwrap();
    std::fs::write(web.join("healthz/unknown"), HEALTH_SHADOW).unwrap();
    std::fs::create_dir_all(web.join("apiary/v1")).unwrap();
    std::fs::write(web.join("apiary/v1/unknown"), APIARY_ASSET).unwrap();
    let state = AppState::new(Catalog::open_in_memory().unwrap(), cache);
    let app = build_router(state, StaticWebRoot::open(web).unwrap());
    (temp, app)
}

async fn body_bytes(response: axum::response::Response) -> axum::body::Bytes {
    response.into_body().collect().await.unwrap().to_bytes()
}

fn hostile_request(method: Method, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("host", "attacker.example")
        .header("x-forwarded-host", "photos.attacker.example")
        .header("x-forwarded-proto", "https")
        .body(Body::empty())
        .unwrap()
}

fn assert_interface_headers(response: &axum::response::Response) {
    assert_eq!(response.headers()[CONTENT_SECURITY_POLICY], CSP);
    assert_eq!(response.headers()[X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(response.headers()[REFERRER_POLICY], "no-referrer");
    assert!(
        response.headers()[CONTENT_SECURITY_POLICY]
            .to_str()
            .unwrap()
            .contains("frame-ancestors 'none'")
    );
}

#[tokio::test]
async fn root_and_nested_interface_routes_serve_the_shell_without_redirects_or_origins() {
    let (_temp, app) = app();
    for uri in ["/", "/gallery/selection-id"] {
        let response = app
            .clone()
            .oneshot(hostile_request(Method::GET, uri))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/html");
        assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
        assert!(response.headers().get(LOCATION).is_none());
        assert_interface_headers(&response);
        let body = String::from_utf8(body_bytes(response).await.to_vec()).unwrap();
        assert_eq!(body, INDEX);
        for forbidden in [
            "http://",
            "https://",
            "attacker.example",
            "photos.attacker.example",
        ] {
            assert!(!body.contains(forbidden), "response echoed {forbidden}");
        }
    }

    let response = app
        .oneshot(hostile_request(Method::HEAD, "/gallery/selection-id"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(LOCATION).is_none());
    assert_interface_headers(&response);
    assert!(body_bytes(response).await.is_empty());
}

#[tokio::test]
async fn immutable_assets_support_etags_and_precompressed_delivery() {
    let (_temp, app) = app();
    let response = app
        .clone()
        .oneshot(hostile_request(Method::GET, "/assets/app-immutable.js"))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_interface_headers(&response);
    let etag = response.headers()[ETAG].clone();
    assert_eq!(body_bytes(response).await, JAVASCRIPT);

    let response = app
        .clone()
        .oneshot(
            Request::get("/assets/app-immutable.js")
                .header(IF_NONE_MATCH, etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(
        response.headers()[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert!(body_bytes(response).await.is_empty());

    let response = app
        .clone()
        .oneshot(
            Request::get("/assets/app-immutable.js")
                .header("accept-encoding", "br")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_ENCODING], "br");
    assert!(
        response.headers()[VARY]
            .to_str()
            .unwrap()
            .contains("accept-encoding")
    );
    assert_eq!(
        response.headers()[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(body_bytes(response).await, BROTLI_JAVASCRIPT);

    let response = app
        .oneshot(
            Request::get("/assets/app-immutable.js")
                .header("accept-encoding", "gzip")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_ENCODING], "gzip");
    assert_eq!(body_bytes(response).await, GZIP_JAVASCRIPT);
}

#[tokio::test]
async fn file_api_and_health_misses_never_fall_back_to_the_interface_shell() {
    let (_temp, app) = app();
    let response = app
        .clone()
        .oneshot(hostile_request(Method::GET, "/missing.txt"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(body_bytes(response).await, INDEX);

    for uri in ["/api/v1/unknown", "/healthz/unknown"] {
        let response = app
            .clone()
            .oneshot(hostile_request(Method::GET, uri))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[CONTENT_TYPE], "application/json");
        assert!(response.headers().get(LOCATION).is_none());
        let body = body_bytes(response).await;
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "notFound");
        assert!(!value.to_string().contains("unknown"));
        assert!(!value.to_string().contains("attacker.example"));
    }

    let response = app
        .oneshot(hostile_request(Method::POST, "/gallery/selection-id"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_ne!(body_bytes(response).await, INDEX);
}

#[tokio::test]
async fn encoded_or_malformed_paths_are_rejected_before_file_or_spa_routing() {
    let (_temp, app) = app();
    for uri in [
        "/missing%2Etxt",
        "/api%2Fv1%2Funknown",
        "/healthz%2Funknown",
        "/%2e%2e/source-original.jpg",
        "/gallery/%ZZ",
    ] {
        let response = app
            .clone()
            .oneshot(hostile_request(Method::GET, uri))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        assert_ne!(body_bytes(response).await, INDEX, "{uri}");
    }
}

#[tokio::test]
async fn encoded_reserved_namespaces_return_the_fixed_api_miss_before_static_lookup() {
    let (_temp, app) = app();
    for (uri, shadow) in [
        ("/%61pi/v1/unknown", API_SHADOW),
        ("/a%70i/v1/unknown", API_SHADOW),
        ("/%68ealthz/unknown", HEALTH_SHADOW),
        ("/h%65althz/unknown", HEALTH_SHADOW),
    ] {
        let response = app
            .clone()
            .oneshot(hostile_request(Method::GET, uri))
            .await
            .unwrap();
        let status = response.status();
        let content_type = response.headers().get(CONTENT_TYPE).cloned();
        let body = body_bytes(response).await;
        assert_ne!(String::from_utf8_lossy(&body), shadow, "{uri}");
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(content_type.unwrap(), "application/json", "{uri}");
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "notFound", "{uri}");
        assert_eq!(value["message"], "That route is unavailable.", "{uri}");
    }

    let response = app
        .oneshot(hostile_request(Method::GET, "/%61piary/v1/unknown"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_bytes(response).await, APIARY_ASSET);
}

#[tokio::test]
async fn oversized_static_paths_are_rejected_with_a_fixed_path_free_response() {
    let (_temp, app) = app();
    let oversized_total = format!("/{}", vec!["a".repeat(240); 18].join("/"));
    let too_many_components = format!("/{}", vec!["a"; 65].join("/"));
    let oversized_component = format!("/{}", "%61".repeat(256));

    for uri in [oversized_total, too_many_components, oversized_component] {
        let response = app
            .clone()
            .oneshot(hostile_request(Method::GET, &uri))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::URI_TOO_LONG);
        let body = body_bytes(response).await;
        assert!(body.is_empty());
        assert!(!String::from_utf8_lossy(&body).contains(&uri));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn configured_web_root_swap_cannot_pin_and_serve_the_source_inode() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web = temp.path().join("web");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&web).unwrap();
    std::fs::write(source.join("index.html"), b"SOURCE ROOT SENTINEL").unwrap();
    std::fs::write(web.join("index.html"), b"VALIDATED WEB ROOT").unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source.clone(),
        web.clone(),
    )
    .unwrap();
    let (state, _) = AppState::open(&config).unwrap();

    std::fs::rename(&web, temp.path().join("validated-web-moved")).unwrap();
    std::fs::rename(&source, &web).unwrap();

    let response = build_router(state, config.static_web_root())
        .oneshot(hostile_request(Method::GET, "/"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_bytes(response).await;
    assert_ne!(body, b"SOURCE ROOT SENTINEL".as_slice());
    assert_eq!(body, b"VALIDATED WEB ROOT".as_slice());
}

#[tokio::test]
async fn nested_fallback_preserves_precompression_and_head_headers() {
    let (_temp, app) = app();
    let response = app
        .clone()
        .oneshot(
            Request::get("/gallery/selection-id")
                .header("accept-encoding", "br")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_ENCODING], "br");
    assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
    assert_eq!(body_bytes(response).await, BROTLI_INDEX);

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::HEAD)
                .uri("/gallery/selection-id")
                .header("accept-encoding", "br")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[CONTENT_ENCODING], "br");
    assert_eq!(
        response.headers()[axum::http::header::CONTENT_LENGTH],
        BROTLI_INDEX.len().to_string()
    );
    assert_eq!(body_bytes(response).await.len(), 0);
}

#[tokio::test]
async fn nested_fallback_preserves_conditional_and_range_headers() {
    let (_temp, app) = app();
    let response = app
        .clone()
        .oneshot(hostile_request(Method::GET, "/gallery/selection-id"))
        .await
        .unwrap();
    let etag = response.headers()[ETAG].clone();

    let response = app
        .clone()
        .oneshot(
            Request::get("/gallery/selection-id")
                .header(IF_NONE_MATCH, etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
    assert!(body_bytes(response).await.is_empty());

    let response = app
        .oneshot(
            Request::get("/gallery/selection-id")
                .header(RANGE, "bytes=0-3")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(response.headers()[CACHE_CONTROL], "no-cache");
    assert_eq!(
        response.headers()[CONTENT_RANGE],
        format!("bytes 0-3/{}", INDEX.len())
    );
    assert_eq!(body_bytes(response).await, &INDEX.as_bytes()[..=3]);
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_asset_never_serves_a_source_sentinel() {
    use std::os::unix::fs::symlink;

    let (temp, app) = app();
    let source_asset = temp.path().join("source-original.jpg");
    std::fs::write(&source_asset, b"SOURCE ORIGINAL ASSET").unwrap();
    let asset = temp.path().join("web/assets/app-immutable.js");
    std::fs::remove_file(&asset).unwrap();
    symlink(&source_asset, &asset).unwrap();

    let response = app
        .oneshot(hostile_request(Method::GET, "/assets/app-immutable.js"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(
        body_bytes(response).await,
        b"SOURCE ORIGINAL ASSET".as_slice()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_index_never_serves_a_source_sentinel() {
    use std::os::unix::fs::symlink;

    let (temp, app) = app();
    let source_index = temp.path().join("source-index.html");
    std::fs::write(&source_index, b"SOURCE ORIGINAL INDEX").unwrap();
    let index = temp.path().join("web/index.html");
    std::fs::remove_file(&index).unwrap();
    symlink(&source_index, &index).unwrap();

    let response = app
        .oneshot(hostile_request(Method::GET, "/gallery/selection-id"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_ne!(
        body_bytes(response).await,
        b"SOURCE ORIGINAL INDEX".as_slice()
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlinked_precompressed_sidecars_never_serve_source_sentinels() {
    use std::os::unix::fs::symlink;

    for (extension, encoding) in [("br", "br"), ("gz", "gzip")] {
        let (temp, app) = app();
        let source_sidecar = temp.path().join(format!("source-sidecar.{extension}"));
        std::fs::write(&source_sidecar, b"SOURCE ORIGINAL SIDECAR").unwrap();
        let sidecar = temp
            .path()
            .join(format!("web/assets/app-immutable.js.{extension}"));
        std::fs::remove_file(&sidecar).unwrap();
        symlink(&source_sidecar, &sidecar).unwrap();

        let response = app
            .oneshot(
                Request::get("/assets/app-immutable.js")
                    .header("accept-encoding", encoding)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{extension}");
        assert!(
            response.headers().get(CONTENT_ENCODING).is_none(),
            "{extension}"
        );
        let body = body_bytes(response).await;
        assert_eq!(body, JAVASCRIPT, "{extension}");
        assert_ne!(body, b"SOURCE ORIGINAL SIDECAR".as_slice(), "{extension}");
    }
}
