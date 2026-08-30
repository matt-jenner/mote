use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use futures_util::stream;
use http_body_util::BodyExt;
use photo_server::{AppState, ServerConfig, build_router};
use serde_json::json;
use tempfile::TempDir;
use tower::ServiceExt;

const MOUNTAIN: &[u8] = include_bytes!("../../../apps/interface/public/demo-photos/mountain.jpg");
const COAST: &[u8] = include_bytes!("../../../apps/interface/public/demo-photos/coast.jpg");

fn app() -> (TempDir, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(source.join("Trips")).unwrap();
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
}

fn app_with_photos() -> (TempDir, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("mountain.jpg"), MOUNTAIN).unwrap();
    std::fs::write(source.join("coast.jpg"), COAST).unwrap();
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
}

async fn json_body(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn create_and_restore_selection_returns_stable_summary() {
    let (temp, app) = app();
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/selections")
                .header("content-type", "application/json")
                .body(Body::from(json!({"path": "Trips"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let summary = json_body(response).await;
    assert_eq!(summary["displayName"], "Trips");
    let selection_id = summary["id"].as_str().unwrap().to_owned();

    drop(app);
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        temp.path().join("photos"),
        temp.path().join("web"),
    )
    .unwrap();
    let (state, _) = AppState::open(&config).unwrap();
    let restored = build_router(state)
        .oneshot(
            Request::get(format!("/api/v1/selections/{selection_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(restored.status(), StatusCode::OK);
    let restored_summary = json_body(restored).await;
    assert_eq!(restored_summary["id"], selection_id);
    assert_eq!(restored_summary["breadcrumbs"][0]["path"], "Trips");
}

#[tokio::test]
async fn invalid_request_limits_are_rejected_before_lookup() {
    let (_temp, app) = app();
    let oversized_query = format!(
        "/api/v1/selections/not-used/events?clientId={}",
        "x".repeat(8192)
    );
    let response = app
        .clone()
        .oneshot(Request::get(oversized_query).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");

    let oversized_id = format!("/api/v1/selections/{}", "x".repeat(129));
    let response = app
        .oneshot(Request::get(oversized_id).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");
}

#[tokio::test]
async fn wall_limit_zero_and_above_maximum_use_specific_error() {
    let (_temp, app) = app();
    for limit in [0, 251] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/selections/not-found/wall?scope=currentFolder&direction=oldestFirst&limit={limit}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(response).await["code"], "invalidLimit");
    }
}

#[tokio::test]
async fn gallery_routes_apply_shared_query_guard_before_lookup() {
    let (_temp, app) = app();
    let cases = [
        Request::post("/api/v1/selections?unexpected=value"),
        Request::get("/api/v1/selections/not-found?unexpected=value"),
        Request::post("/api/v1/selections/not-found/interaction?unexpected=value"),
    ];
    for builder in cases {
        let response = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(response).await["code"], "invalidRequest");
    }

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/selections/not-found?scope=currentFolder&scope=currentFolder")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/selections/not-found/wall?a=1&b=2&c=3&d=4&e=5&f=6&g=7&h=8&i=9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");
}

#[tokio::test]
async fn selection_bodies_are_rejected_at_the_64k_boundary() {
    let (_temp, app) = app();
    for body in [
        Body::from(Bytes::from(vec![b'x'; 64 * 1024 + 1])),
        Body::from(Bytes::from(vec![b'x'; 2 * 1024 * 1024 + 1])),
        Body::from_stream(stream::iter(vec![
            Ok::<_, std::convert::Infallible>(Bytes::from(vec![b'x'; 32 * 1024])),
            Ok(Bytes::from(vec![b'x'; 32 * 1024 + 1])),
        ])),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/v1/selections")
                    .header("content-type", "application/json")
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(response).await["code"], "invalidRequest");
    }

    let response = app
        .oneshot(
            Request::post("/api/v1/selections/not-found/interaction")
                .header("content-type", "application/json")
                .body(Body::from(Bytes::from(vec![b'x'; 64 * 1024 + 1])))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");
}

#[tokio::test]
async fn unavailable_child_selection_is_not_reported_as_mount_failure() {
    let (_temp, app) = app();
    let response = app
        .oneshot(
            Request::post("/api/v1/selections")
                .header("content-type", "application/json")
                .body(Body::from(json!({"path": "Trips/Missing"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(json_body(response).await["code"], "folderUnavailable");
}

#[tokio::test]
async fn unknown_selection_errors_do_not_echo_the_path_or_id() {
    let (_temp, app) = app();
    let response = app
        .oneshot(
            Request::get("/api/v1/selections/does-not-exist")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = json_body(response).await;
    assert_eq!(body["code"], "notFound");
    assert!(!body.to_string().contains("does-not-exist"));
}

#[tokio::test]
async fn wall_paging_and_route_cursor_scope_direction_validation_work() {
    let (_temp, app) = app_with_photos();
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
    let summary = json_body(created).await;
    let id = summary["id"].as_str().unwrap().to_owned();

    let first = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!(
                        "/api/v1/selections/{id}/wall?scope=currentFolder&direction=oldestFirst&limit=1"
                    ))
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let value = json_body(response).await;
            if value["items"].as_array().is_some_and(|items| items.len() == 1) {
                break value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("wall scan did not produce a page");
    let cursor = first["nextCursor"].as_str().unwrap().to_owned();
    let second = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/wall?scope=currentFolder&direction=oldestFirst&limit=1&cursor={cursor}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let second = json_body(second).await;
    assert_eq!(second["items"].as_array().unwrap().len(), 1);
    assert_ne!(first["items"][0]["id"], second["items"][0]["id"]);

    for (scope, direction) in [
        ("includeSubfolders", "oldestFirst"),
        ("currentFolder", "newestFirst"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/selections/{id}/wall?scope={scope}&direction={direction}&limit=1&cursor={cursor}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(response).await["code"], "invalidCursor");
    }
}

#[tokio::test]
async fn route_rejects_a_cursor_over_2048_bytes_before_lookup() {
    let (_temp, app) = app();
    let response = app
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/not-found/wall?scope=currentFolder&direction=oldestFirst&limit=1&cursor={}",
                "x".repeat(2049)
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");
}
