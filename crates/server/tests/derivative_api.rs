use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_server::{AppState, ServerConfig, build_router};
use serde_json::json;
use std::time::Duration;
use tempfile::TempDir;
use tower::ServiceExt;

const JPEG: &[u8] = include_bytes!("../../../apps/interface/public/demo-photos/mountain.jpg");

fn make_app() -> (TempDir, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web = temp.path().join("web");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&web).unwrap();
    std::fs::write(source.join("photo.jpg"), JPEG).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        web.clone(),
    )
    .unwrap();
    let (state, _) = AppState::open(&config).unwrap();
    (temp, build_router(state, web))
}

async fn body(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn selection_and_asset(app: &axum::Router) -> (String, String) {
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
    let asset_id = tokio::time::timeout(Duration::from_secs(10), async {
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
                    break item["id"].as_str().unwrap().to_owned();
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    (selection_id, asset_id)
}

#[tokio::test]
async fn derivative_route_delivers_managed_jpeg_with_immutable_headers() {
    let (temp, app) = make_app();
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
    std::fs::remove_file(temp.path().join("photos/photo.jpg")).unwrap();
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
    let catalog = photo_catalog::Catalog::open(&temp.path().join("data/catalog.sqlite")).unwrap();
    let record = catalog.find_derivative_by_cache_key(&key).unwrap().unwrap();
    let managed_path = temp.path().join("cache").join(record.relative_cache_path);

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

    // A catalogued sparse file above the managed derivative ceiling must be
    // rejected from GET without reading its apparent length into memory.
    let managed_file = std::fs::OpenOptions::new()
        .write(true)
        .open(&managed_path)
        .unwrap();
    managed_file.set_len(128 * 1024 * 1024).unwrap();
    drop(managed_file);
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    std::fs::write(&managed_path, &bytes).unwrap();
    let mut malformed = vec![0xff_u8, 0xd8, 0xff];
    malformed.resize(bytes.len(), 0);
    std::fs::write(&managed_path, malformed).unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    std::fs::write(&managed_path, &bytes).unwrap();

    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{key}"))
                .header("range", "bytes=0-2")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    std::fs::remove_file(&managed_path).unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(body(response).await["code"], "notFound");

    #[cfg(unix)]
    {
        let outside = temp.path().join("outside.jpg");
        std::fs::write(&outside, JPEG).unwrap();
        std::os::unix::fs::symlink(&outside, &managed_path).unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::get(format!("/api/v1/derivatives/{key}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(body(response).await["code"], "notFound");
    }

    // A catalog row must not turn an opaque derivative id into a traversal
    // primitive. Bypass the typed catalog writer to model a legacy/corrupt row
    // and prove the HTTP boundary still refuses to open outside the cache.
    let outside = temp.path().join("outside-row.jpg");
    std::fs::write(&outside, JPEG).unwrap();
    drop(catalog);
    let connection = rusqlite::Connection::open(temp.path().join("data/catalog.sqlite")).unwrap();
    connection
        .execute(
            "UPDATE derivatives SET relative_cache_path = ?1 WHERE cache_key = ?2",
            rusqlite::params!["../outside-row.jpg", key],
        )
        .unwrap();
    drop(connection);
    let response = app
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{key}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert_eq!(std::fs::read(&outside).unwrap(), JPEG);
}

#[tokio::test]
async fn derivative_route_rejects_oversized_decoded_id_and_query_before_lookup() {
    let (_temp, app) = make_app();
    let oversized_id = "a".repeat(513);
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/derivatives/{oversized_id}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(response).await["code"], "invalidRequest");

    let oversized_query = "x=".to_owned() + &"a".repeat(8_200);
    let response = app
        .clone()
        .oneshot(
            Request::get(format!("/api/v1/derivatives/missing?{oversized_query}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(response).await["code"], "invalidRequest");

    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/selections/missing/derivatives")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "scope": "currentFolder",
                        "request": {
                            "assetIds": vec!["x"; 251],
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
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(response).await["code"], "invalidRequest");
}

#[tokio::test]
async fn derivative_request_exposes_invalid_unavailable_and_failed_envelopes() {
    let (temp, app) = make_app();
    let (selection_id, asset_id) = selection_and_asset(&app).await;

    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/selections/{selection_id}/derivatives"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "scope": "currentFolder",
                        "request": {
                            "assetIds": [],
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
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body(response).await["code"], "invalidRequest");

    std::fs::remove_file(temp.path().join("photos/photo.jpg")).unwrap();
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
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body(response).await["code"], "derivativeUnavailable");

    let (temp2, app2) = make_app();
    let (selection_id, asset_id) = selection_and_asset(&app2).await;
    std::fs::write(temp2.path().join("photos/photo.jpg"), b"not a jpeg").unwrap();
    let response = app2
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
    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(body(response).await["code"], "derivativeFailed");
}
