use axum::body::Body;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_ENCODING, CONTENT_SECURITY_POLICY, CONTENT_TYPE, ETAG, IF_NONE_MATCH,
    LOCATION, REFERRER_POLICY, VARY, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use photo_catalog::Catalog;
use photo_server::{AppState, build_router};
use tempfile::TempDir;
use tower::ServiceExt;

const INDEX: &str = r#"<!doctype html><title>Photo Viewer</title><script type="module" src="/assets/app-immutable.js"></script>"#;
const JAVASCRIPT: &str = "globalThis.photoViewer = true;";
const BROTLI_JAVASCRIPT: &[u8] = b"precompressed-javascript";
const CSP: &str = "default-src 'self'; img-src 'self' data: blob:; script-src 'self'; style-src 'self' 'unsafe-inline'; connect-src 'self'; object-src 'none'; base-uri 'self'; frame-ancestors 'none'";

fn app() -> (TempDir, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let web = temp.path().join("web");
    let assets = web.join("assets");
    let cache = temp.path().join("cache");
    std::fs::create_dir_all(&assets).unwrap();
    std::fs::create_dir(&cache).unwrap();
    std::fs::write(web.join("index.html"), INDEX).unwrap();
    std::fs::write(assets.join("app-immutable.js"), JAVASCRIPT).unwrap();
    std::fs::write(assets.join("app-immutable.js.br"), BROTLI_JAVASCRIPT).unwrap();
    let state = AppState::new(Catalog::open_in_memory().unwrap(), cache);
    let app = build_router(state, web);
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
