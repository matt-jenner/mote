use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_server::{AppState, ServerConfig, build_router};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn events_route_declares_sse_headers() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        temp.path().join("web"),
    )
    .unwrap();
    let (state, _) = AppState::open(&config).unwrap();
    let app = build_router(state);
    let created = app
        .clone()
        .oneshot(
            Request::post("/api/v1/selections")
                .header("content-type", "application/json")
                .body(Body::from(json!({"path": ""}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let value: serde_json::Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = value["id"].as_str().unwrap();
    let response = app
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId=browser&scope=currentFolder"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(response.headers()["cache-control"], "no-cache");
    assert_eq!(response.headers()["x-accel-buffering"], "no");
}
