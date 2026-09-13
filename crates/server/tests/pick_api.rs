use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_server::{AppState, ServerConfig, build_router};
use serde_json::{Value, json};
use std::time::Duration;
use tempfile::TempDir;
use tower::ServiceExt;

const JPEG: &[u8] = include_bytes!("../../../apps/interface/public/demo-photos/mountain.jpg");

fn make_app() -> (TempDir, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web = temp.path().join("web");
    std::fs::create_dir_all(source.join("Family")).unwrap();
    std::fs::create_dir_all(source.join("Trips")).unwrap();
    std::fs::create_dir(&web).unwrap();
    std::fs::write(source.join("Family/one.jpg"), JPEG).unwrap();
    std::fs::write(source.join("Family/two.jpg"), JPEG).unwrap();
    std::fs::write(source.join("Trips/foreign.jpg"), JPEG).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        web,
    )
    .unwrap();
    let (state, _) = AppState::open(&config).unwrap();
    (temp, build_router(state, config.static_web_root()))
}

async fn json_body(response: axum::response::Response) -> Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}

async fn select(app: &axum::Router, path: &str) -> String {
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/v1/selections")
                .header("content-type", "application/json")
                .body(Body::from(json!({"path": path}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    json_body(response).await["id"].as_str().unwrap().to_owned()
}

async fn assets(app: &axum::Router, selection_id: &str, expected: usize) -> Vec<Value> {
    tokio::time::timeout(Duration::from_secs(10), async {
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
                let value = json_body(response).await;
                if value["items"].as_array().is_some_and(|items| items.len() == expected) {
                    break value["items"].as_array().unwrap().clone();
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}

fn resolve_request(selection_id: &str, asset_ids: Vec<String>) -> Request<Body> {
    Request::post(format!("/api/v1/selections/{selection_id}/assets"))
        .header("content-type", "application/json")
        .body(Body::from(json!({"assetIds": asset_ids}).to_string()))
        .unwrap()
}

#[tokio::test]
async fn selection_assets_preserve_request_order_and_reject_foreign_ids() {
    let (_temp, app) = make_app();
    let family = select(&app, "Family").await;
    let trips = select(&app, "Trips").await;
    let family_assets = assets(&app, &family, 2).await;
    let trip_assets = assets(&app, &trips, 1).await;
    let first = family_assets[0]["id"].as_str().unwrap().to_owned();
    let second = family_assets[1]["id"].as_str().unwrap().to_owned();
    let foreign = trip_assets[0]["id"].as_str().unwrap().to_owned();

    let response = app
        .clone()
        .oneshot(resolve_request(
            &family,
            vec![second.clone(), first.clone()],
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let resolved = json_body(response).await;
    assert_eq!(resolved[0]["id"], second);
    assert_eq!(resolved[1]["id"], first);
    assert!(resolved.as_array().unwrap().iter().all(|asset| {
        asset.get("path").is_none()
            && asset.get("sourcePath").is_none()
            && !asset
                .to_string()
                .contains(_temp.path().to_string_lossy().as_ref())
    }));

    let response = app
        .clone()
        .oneshot(resolve_request(&family, vec![foreign]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");
}

#[tokio::test]
async fn selection_assets_reject_unbounded_or_unknown_request_shapes() {
    let (_temp, app) = make_app();
    let family = select(&app, "Family").await;
    let response = app
        .clone()
        .oneshot(resolve_request(&family, vec!["id".to_owned(); 251]))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let response = app
        .oneshot(
            Request::post(format!("/api/v1/selections/{family}/assets"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"assetIds": [], "path": "/private/photos"}).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        !json_body(response)
            .await
            .to_string()
            .contains("/private/photos")
    );
}
