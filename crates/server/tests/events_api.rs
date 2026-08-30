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

#[tokio::test]
async fn future_replay_uses_authoritative_head_and_sse_framing() {
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
    let value: serde_json::Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = value["id"].as_str().unwrap();
    let response = app
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId=browser&scope=currentFolder&afterEventId=0"
            ))
            .header("last-event-id", u64::MAX.to_string())
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut body = response.into_body();
    let frame = body.frame().await.unwrap().unwrap();
    let data = frame.into_data().unwrap();
    let text = String::from_utf8(data.to_vec()).unwrap();
    assert!(text.starts_with("event: wallUpdate\nid: "));
    assert!(text.contains("\ndata: "));
    assert!(text.ends_with("\n\n"));
    assert!(text.contains("resyncRequired"));
}

#[tokio::test]
async fn interaction_for_an_unknown_client_is_an_idempotent_no_content() {
    let (_temp, app) = {
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
        (temp, build_router(state))
    };
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
    let value: serde_json::Value =
        serde_json::from_slice(&created.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = value["id"].as_str().unwrap();
    let response = app
        .oneshot(
            Request::post(format!("/api/v1/selections/{id}/interaction"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "clientId": "unknown",
                        "scope": "currentFolder",
                        "state": "active"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn event_query_and_identifier_limits_use_invalid_request() {
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
    for query in [
        "clientId=x&scope=currentFolder&scope=currentFolder",
        &format!("clientId={}&scope=currentFolder", "x".repeat(129)),
        "clientId=x&scope=currentFolder&afterEventId=123456789012345678901",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/selections/not-found/events?{query}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body: serde_json::Value =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(body["code"], "invalidRequest");
    }
}
