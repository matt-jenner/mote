use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_server::{AppState, ServerConfig, build_router};
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;
use tower::ServiceExt;

const JPEG: &[u8] = include_bytes!("../../../apps/interface/public/demo-photos/mountain.jpg");

fn app() -> (TempDir, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(source.join("photo.jpg"), JPEG).unwrap();
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

async fn body(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

#[tokio::test]
async fn derivative_route_delivers_managed_jpeg_with_immutable_headers() {
    let (_temp, app) = app();
    let selected = app
        .clone()
        .oneshot(
            Request::post("/api/v1/selections")
                .header("content-type", "application/json")
                .body(Body::from(json!({"path": ""}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(selected.status(), StatusCode::CREATED);
    let selection_id = body(selected).await["id"].as_str().unwrap().to_owned();

    let (asset_id, key) = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!(
                        "/api/v1/selections/{selection_id}/wall?scope=currentFolder&direction=oldestFirst&limit=50"
                    ))
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            if response.status() == StatusCode::OK {
                let value = body(response).await;
                if let Some(item) = value["items"].as_array().and_then(|items| items.first()) {
                    break (
                        item["id"].as_str().unwrap().to_owned(),
                        item["wallThumbnail"]["key"].as_str().map(str::to_owned),
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/selections/{selection_id}/derivatives"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "scope": "currentFolder",
                        "request": {
                            "assetIds": [asset_id],
                            "priority": "visible",
                            "kind": "wallThumbnail"
                        }
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);

    let key = if let Some(key) = key {
        key
    } else {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/selections/{selection_id}/wall?scope=currentFolder&direction=oldestFirst&limit=50"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        body(response).await["items"][0]["wallThumbnail"]["key"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "image/jpeg");
    assert_eq!(
        response.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert!(
        response.headers()["etag"]
            .to_str()
            .unwrap()
            .starts_with('"')
    );
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
    let etag = response.headers()["etag"].clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&bytes[..3], &[0xff, 0xd8, 0xff]);

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{key}"))
                .header("if-none-match", etag)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);

    let response = app
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{key}"))
                .header("range", "bytes=0-2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}
