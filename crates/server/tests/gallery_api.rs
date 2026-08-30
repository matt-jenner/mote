use axum::body::{Body, Bytes};
use axum::http::{Request, StatusCode};
use futures_util::stream;
use http_body_util::BodyExt;
use photo_app_service::{GalleryScope, WallUpdate};
use photo_server::{AppState, ServerConfig, build_router};
use serde_json::json;
use std::path::Path;
use tempfile::TempDir;
use tower::ServiceExt;

const MOUNTAIN: &[u8] = include_bytes!("../../../apps/interface/public/demo-photos/mountain.jpg");

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

fn app_with_photos() -> (TempDir, AppState, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    for (name, captured_at) in [
        ("oldest.jpg", "2020:01:01 00:00:00"),
        ("middle.jpg", "2021:01:01 00:00:00"),
        ("newest.jpg", "2022:01:01 00:00:00"),
    ] {
        std::fs::write(source.join(name), jpeg_with_capture_date(captured_at)).unwrap();
    }
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        temp.path().join("web"),
    )
    .unwrap();
    let (state, _) = AppState::open(&config).unwrap();
    (temp, state.clone(), build_router(state))
}

fn jpeg_with_capture_date(captured_at: &str) -> Vec<u8> {
    assert_eq!(captured_at.len(), 19);
    let mut exif = Vec::new();
    exif.extend_from_slice(b"Exif\0\0");
    exif.extend_from_slice(b"MM");
    exif.extend_from_slice(&0x002a_u16.to_be_bytes());
    exif.extend_from_slice(&8_u32.to_be_bytes());
    exif.extend_from_slice(&1_u16.to_be_bytes());
    exif.extend_from_slice(&0x8769_u16.to_be_bytes());
    exif.extend_from_slice(&4_u16.to_be_bytes());
    exif.extend_from_slice(&1_u32.to_be_bytes());
    exif.extend_from_slice(&26_u32.to_be_bytes());
    exif.extend_from_slice(&0_u32.to_be_bytes());
    exif.extend_from_slice(&1_u16.to_be_bytes());
    exif.extend_from_slice(&0x9003_u16.to_be_bytes());
    exif.extend_from_slice(&2_u16.to_be_bytes());
    exif.extend_from_slice(&20_u32.to_be_bytes());
    exif.extend_from_slice(&44_u32.to_be_bytes());
    exif.extend_from_slice(&0_u32.to_be_bytes());
    exif.extend_from_slice(captured_at.as_bytes());
    exif.push(0);
    assert_eq!(exif.len(), 70);

    let segment_length = u16::try_from(exif.len() + 2).unwrap();
    let mut jpeg = Vec::with_capacity(MOUNTAIN.len() + exif.len() + 6);
    jpeg.extend_from_slice(&[0xff, 0xd8, 0xff, 0xe1]);
    jpeg.extend_from_slice(&segment_length.to_be_bytes());
    jpeg.extend_from_slice(&exif);
    jpeg.extend_from_slice(&MOUNTAIN[2..]);
    jpeg
}

async fn wait_for_metadata_settled(
    gallery: &photo_app_service::GalleryEngine,
    selection: &photo_app_service::GallerySelection,
    client_id: &str,
) -> photo_app_service::SelectionEventSubscription {
    let mut updates = gallery.subscribe(
        selection,
        client_id.to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    gallery.ensure_running(selection).await.unwrap();
    loop {
        let event = updates
            .recv()
            .await
            .expect("selection scan ended before metadata settlement");
        if matches!(event.update, WallUpdate::MetadataSettled { .. }) {
            break;
        }
    }
    updates
}

fn app_with_state() -> (TempDir, AppState, axum::Router) {
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
    let app = build_router(state.clone());
    (temp, state, app)
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
        .clone()
        .oneshot(Request::get(oversized_id).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/folders?a=1&b=2&c=3&d=4&e=5&f=6&g=7&h=8")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .clone()
        .oneshot(
            Request::get("/api/v1/folders?a=1&b=2&c=3&d=4&e=5&f=6&g=7&h=8&i=9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");
    let oversized_target = (b'a'..=b'h')
        .enumerate()
        .map(|(index, _)| format!("p{index}={}", "x".repeat(1_030)))
        .collect::<Vec<_>>()
        .join("&");
    assert!(oversized_target.len() > 8_192);
    let response = app
        .oneshot(
            Request::get(format!("/api/v1/folders?{oversized_target}"))
                .body(Body::empty())
                .unwrap(),
        )
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
}

#[tokio::test]
async fn selection_bodies_are_rejected_at_the_64k_boundary() {
    let (_temp, state, app) = app_with_state();
    let gallery = state.gallery_for_test().unwrap();
    assert_eq!(gallery.runtime_count_for_test(), 0);
    let valid_create = |padding: usize| {
        serde_json::to_vec(&json!({
            "path": "",
            "padding": "x".repeat(padding)
        }))
        .unwrap()
    };
    let chunked = valid_create(64 * 1024);
    for body in [
        Body::from(Bytes::from(valid_create(64 * 1024))),
        Body::from(Bytes::from(valid_create(2 * 1024 * 1024))),
        Body::from_stream(stream::iter(vec![
            Ok::<_, std::convert::Infallible>(Bytes::copy_from_slice(&chunked[..32 * 1024])),
            Ok(Bytes::copy_from_slice(&chunked[32 * 1024..])),
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
        assert_eq!(gallery.runtime_count_for_test(), 0);
    }

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
    let id = json_body(created).await["id"].as_str().unwrap().to_owned();
    let runtime_count = gallery.runtime_count_for_test();
    let interaction = serde_json::to_vec(&json!({
        "clientId": "client",
        "scope": "currentFolder",
        "state": "idle",
        "padding": "x".repeat(64 * 1024)
    }))
    .unwrap();
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/selections/{id}/interaction"))
                .header("content-type", "application/json")
                .body(Body::from(interaction))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(json_body(response).await["code"], "invalidRequest");
    assert_eq!(gallery.runtime_count_for_test(), runtime_count);
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
    let (_temp, state, app) = app_with_photos();
    let gallery = state.gallery_for_test().unwrap();
    let summary = gallery.select_relative(Path::new(".")).await.unwrap();
    let id = summary.id.clone();
    let selection = gallery.resolve_selection(&id).unwrap();
    let _settled_subscription =
        wait_for_metadata_settled(&gallery, &selection, "wall-settler").await;

    let mut expected_oldest_cursor = None;
    let mut expected_oldest_terminal_cursor = None;
    let mut expected_newest_cursor = None;
    let mut expected_newest_terminal_cursor = None;
    for _ in 0..100 {
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let response = app
                    .clone()
                    .oneshot(
                        Request::get(format!(
                            "/api/v1/selections/{id}/wall?scope=currentFolder&direction=oldestFirst&limit=2"
                        ))
                        .body(Body::empty())
                        .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let value = json_body(response).await;
                if value["orderState"] == "settled"
                    && value["items"].as_array().is_some_and(|items| items.len() == 2)
                    && value["nextCursor"].as_str().is_some()
                {
                    break value;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wall scan did not produce a settled page with a cursor");
        assert_eq!(
            first["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["displayName"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["oldest.jpg", "middle.jpg"]
        );
        let cursor = first["nextCursor"].as_str().unwrap().to_owned();
        if let Some(expected) = &expected_oldest_cursor {
            assert_eq!(&cursor, expected);
        } else {
            expected_oldest_cursor = Some(cursor.clone());
        }

        let second_response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/selections/{id}/wall?scope=currentFolder&direction=oldestFirst&limit=2&cursor={cursor}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second_response.status(), StatusCode::OK);
        let second = json_body(second_response).await;
        assert_eq!(second["orderState"], "settled");
        assert_eq!(
            second["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["displayName"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["newest.jpg"]
        );
        let second_ids = second["items"].as_array().unwrap();
        let first_ids = first["items"].as_array().unwrap();
        let mut ids = std::collections::HashSet::new();
        for item in first_ids.iter().chain(second_ids.iter()) {
            assert!(ids.insert(item["id"].as_str().unwrap()));
        }
        assert_eq!(ids.len(), 3);
        let terminal_cursor = second["nextCursor"].as_str().unwrap().to_owned();
        if let Some(expected) = &expected_oldest_terminal_cursor {
            assert_eq!(&terminal_cursor, expected);
        } else {
            expected_oldest_terminal_cursor = Some(terminal_cursor.clone());
        }
        let terminal_response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/selections/{id}/wall?scope=currentFolder&direction=oldestFirst&limit=2&cursor={terminal_cursor}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(terminal_response.status(), StatusCode::OK);
        let terminal = json_body(terminal_response).await;
        assert!(terminal["items"].as_array().unwrap().is_empty());
        assert_eq!(terminal["nextCursor"], serde_json::Value::Null);

        let newest = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let response = app
                    .clone()
                    .oneshot(
                        Request::get(format!(
                            "/api/v1/selections/{id}/wall?scope=currentFolder&direction=newestFirst&limit=2"
                        ))
                        .body(Body::empty())
                        .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                let value = json_body(response).await;
                if value["orderState"] == "settled"
                    && value["items"].as_array().is_some_and(|items| items.len() == 2)
                    && value["nextCursor"].as_str().is_some()
                {
                    break value;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("wall scan did not produce a settled newest page with a cursor");
        assert_eq!(
            newest["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["displayName"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["newest.jpg", "middle.jpg"]
        );
        let newest_cursor = newest["nextCursor"].as_str().unwrap().to_owned();
        if let Some(expected) = &expected_newest_cursor {
            assert_eq!(&newest_cursor, expected);
        } else {
            expected_newest_cursor = Some(newest_cursor.clone());
        }
        let newest_second_response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/selections/{id}/wall?scope=currentFolder&direction=newestFirst&limit=2&cursor={newest_cursor}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(newest_second_response.status(), StatusCode::OK);
        let newest_second = json_body(newest_second_response).await;
        assert_eq!(newest_second["orderState"], "settled");
        assert_eq!(
            newest_second["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["displayName"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["oldest.jpg"]
        );
        let mut newest_ids = std::collections::HashSet::new();
        for item in newest["items"]
            .as_array()
            .unwrap()
            .iter()
            .chain(newest_second["items"].as_array().unwrap().iter())
        {
            assert!(newest_ids.insert(item["id"].as_str().unwrap()));
        }
        assert_eq!(newest_ids.len(), 3);
        let newest_terminal_cursor = newest_second["nextCursor"].as_str().unwrap().to_owned();
        if let Some(expected) = &expected_newest_terminal_cursor {
            assert_eq!(&newest_terminal_cursor, expected);
        } else {
            expected_newest_terminal_cursor = Some(newest_terminal_cursor.clone());
        }
        let newest_terminal_response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/selections/{id}/wall?scope=currentFolder&direction=newestFirst&limit=2&cursor={newest_terminal_cursor}"
                ))
                .body(Body::empty())
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(newest_terminal_response.status(), StatusCode::OK);
        let newest_terminal = json_body(newest_terminal_response).await;
        assert!(newest_terminal["items"].as_array().unwrap().is_empty());
        assert_eq!(newest_terminal["nextCursor"], serde_json::Value::Null);
    }

    let cursor = expected_oldest_cursor.unwrap();
    for (scope, direction) in [
        ("includeSubfolders", "oldestFirst"),
        ("currentFolder", "newestFirst"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::get(format!(
                    "/api/v1/selections/{id}/wall?scope={scope}&direction={direction}&limit=2&cursor={cursor}"
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
