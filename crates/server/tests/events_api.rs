use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_app_service::{GalleryScope, OrderState, WallUpdate};
use photo_server::{AppState, ServerConfig, build_router};
use serde_json::json;
use tokio::sync::mpsc;
use tower::ServiceExt;

async fn next_sse_frame(body: &mut Body) -> String {
    loop {
        let frame = body.frame().await.unwrap().unwrap();
        if let Ok(data) = frame.into_data() {
            return String::from_utf8(data.to_vec()).unwrap();
        }
    }
}

async fn create_selection(app: &axum::Router, path: &str) -> String {
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
    let value: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    value["id"].as_str().unwrap().to_owned()
}

fn test_state() -> (tempfile::TempDir, AppState, axum::Router) {
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
    let app = build_router(state.clone());
    (temp, state, app)
}

fn nested_state() -> (tempfile::TempDir, AppState, axum::Router) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("Parent/Child")).unwrap();
    std::fs::write(
        source.join("Parent/parent.jpg"),
        include_bytes!("../../../apps/interface/public/demo-photos/mountain.jpg"),
    )
    .unwrap();
    std::fs::write(
        source.join("Parent/Child/child.jpg"),
        include_bytes!("../../../apps/interface/public/demo-photos/coast.jpg"),
    )
    .unwrap();
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

async fn event_body(app: &axum::Router, id: &str, client: &str, scope: &str, after: u64) -> Body {
    app.clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId={client}&scope={scope}&afterEventId={after}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap()
        .into_body()
}

fn progress(selection_id: &str, generation: u64) -> WallUpdate {
    WallUpdate::Progress {
        selection_id: selection_id.to_owned(),
        generation,
        progress: Default::default(),
    }
}

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
        .clone()
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
    let gallery = state.gallery_for_test().unwrap();
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
    let selection = gallery.resolve_selection(&id).unwrap();
    let response = app
        .clone()
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
    let head = sse_id(&text);
    let next = gallery
        .publish_update_for_test(&selection, progress(&id, 1))
        .await;
    let live = next_sse_frame(&mut body).await;
    assert_eq!(sse_id(&live), next.id);
    assert!(next.id > head);
    drop(body);

    let mut reconnect = event_body(&app, &id, "reconnect", "currentFolder", head).await;
    assert_eq!(sse_id(&next_sse_frame(&mut reconnect).await), next.id);
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
        "clientId=x&scope=currentFolder&afterEventId=000000000000000000001",
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

#[tokio::test]
async fn http_replay_supports_header_query_and_old_history_recovery() {
    let (_temp, state, app) = test_state();
    let gallery = state.gallery_for_test().unwrap();
    let id = create_selection(&app, "").await;
    let selection = gallery.resolve_selection(&id).unwrap();
    let base = gallery.current_event_id_for_test(&selection);
    let retained = gallery
        .publish_update_for_test(&selection, progress(&id, 1))
        .await;

    let response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId=normal&scope=currentFolder&afterEventId={base}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let mut normal = response.into_body();
    let first = next_sse_frame(&mut normal).await;
    let first_id = sse_id(&first);
    assert_eq!(first_id, retained.id);
    assert!(first_id > base);
    drop(normal);

    let retained_query = gallery
        .publish_update_for_test(&selection, progress(&id, 2))
        .await;
    let response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId=query&scope=currentFolder&afterEventId={first_id}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let mut query = response.into_body();
    let query_id = sse_id(&next_sse_frame(&mut query).await);
    assert_eq!(query_id, retained_query.id);
    assert!(query_id > first_id);
    drop(query);

    let retained_header = gallery
        .publish_update_for_test(&selection, progress(&id, 3))
        .await;
    let response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId=header&scope=currentFolder"
            ))
            .header("last-event-id", query_id.to_string())
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let mut header = response.into_body();
    let header_id = sse_id(&next_sse_frame(&mut header).await);
    assert_eq!(header_id, retained_header.id);
    assert!(header_id > query_id);
    drop(header);

    let header_base = gallery.current_event_id_for_test(&selection);
    let response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId=precedence&scope=currentFolder&afterEventId=0"
            ))
            .header("last-event-id", header_base.to_string())
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let mut precedence = response.into_body();
    gallery
        .publish_update_for_test(&selection, progress(&id, 3))
        .await;
    let precedence_id = sse_id(&next_sse_frame(&mut precedence).await);
    assert!(
        precedence_id > header_base,
        "header must override query replay"
    );
    drop(precedence);

    for generation in 4..=300 {
        gallery
            .publish_update_for_test(&selection, progress(&id, generation))
            .await;
    }
    let response = app
        .clone()
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId=old&scope=currentFolder&afterEventId=0"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let mut old = response.into_body();
    let resync = next_sse_frame(&mut old).await;
    assert!(resync.contains("resyncRequired"));
    let watermark = sse_id(&resync);
    let next = gallery
        .publish_update_for_test(&selection, progress(&id, 301))
        .await;
    assert_eq!(sse_id(&next_sse_frame(&mut old).await), next.id);
    assert!(next.id > watermark);
}

fn sse_id(frame: &str) -> u64 {
    frame
        .lines()
        .find_map(|line| line.strip_prefix("id: "))
        .unwrap()
        .parse()
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn sse_heartbeat_is_a_framed_comment_after_fifteen_seconds() {
    let (_temp, state, app) = test_state();
    let gallery = state.gallery_for_test().unwrap();
    let id = create_selection(&app, "").await;
    let selection = gallery.resolve_selection(&id).unwrap();
    let base = gallery.current_event_id_for_test(&selection);
    let response = app
        .oneshot(
            Request::get(format!(
                "/api/v1/selections/{id}/events?clientId=heartbeat&scope=currentFolder&afterEventId={base}"
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    let mut body = response.into_body();
    let (sender, mut received) = mpsc::unbounded_channel();
    let reader = tokio::spawn(async move {
        loop {
            let frame = next_sse_frame(&mut body).await;
            sender.send(frame).unwrap();
        }
    });
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    while received.try_recv().is_ok() {}
    tokio::time::advance(std::time::Duration::from_millis(14_999)).await;
    tokio::task::yield_now().await;
    assert!(
        received.try_recv().is_err(),
        "heartbeat arrived before 15 seconds"
    );
    tokio::time::advance(std::time::Duration::from_millis(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(received.recv().await.unwrap(), ": heartbeat\n\n");
    reader.abort();
}

#[tokio::test]
async fn slow_http_consumer_gets_one_lag_resync_then_monotonic_live_event() {
    let (_temp, state, app) = test_state();
    let gallery = state.gallery_for_test().unwrap();
    let id = create_selection(&app, "").await;
    let selection = gallery.resolve_selection(&id).unwrap();
    let base = gallery.current_event_id_for_test(&selection);
    let mut body = event_body(&app, &id, "slow", "currentFolder", base).await;
    for generation in 0..300 {
        gallery
            .publish_update_for_test(&selection, progress(&id, generation))
            .await;
    }
    let resync = next_sse_frame(&mut body).await;
    assert!(resync.contains("resyncRequired"));
    let watermark = sse_id(&resync);
    let next = gallery
        .publish_update_for_test(&selection, progress(&id, 300))
        .await;
    assert_eq!(sse_id(&next_sse_frame(&mut body).await), next.id);
    assert!(next.id > watermark);
    drop(body);
}

#[tokio::test]
async fn dropping_http_event_body_removes_hosted_demand_and_lease() {
    let (_temp, state, app) = test_state();
    let gallery = state.gallery_for_test().unwrap();
    let id = create_selection(&app, "").await;
    let selection = gallery.resolve_selection(&id).unwrap();
    let base = gallery.current_event_id_for_test(&selection);
    let body = event_body(&app, &id, "drop", "includeSubfolders", base).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(gallery.scheduler_permits_for_test(), 1);
    assert_eq!(
        gallery.aggregate_scope_for_test(&selection).await,
        GalleryScope::IncludeSubfolders
    );
    drop(body);
    for _ in 0..6 {
        tokio::task::yield_now().await;
    }
    assert_eq!(gallery.scheduler_permits_for_test(), 4);
    assert_eq!(
        gallery.aggregate_scope_for_test(&selection).await,
        GalleryScope::CurrentFolder
    );
}

#[tokio::test]
async fn http_scopes_filter_assets_and_broader_demand_ends_with_broader_body() {
    let (_temp, state, app) = nested_state();
    let gallery = state.gallery_for_test().unwrap();
    let id = create_selection(&app, "Parent").await;
    let wall = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!(
                        "/api/v1/selections/{id}/wall?scope=includeSubfolders&direction=oldestFirst&limit=10"
                    ))
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(
                &response.into_body().collect().await.unwrap().to_bytes(),
            )
            .unwrap();
            if value["items"].as_array().is_some_and(|items| items.len() == 2) {
                break value;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("nested wall did not settle");
    let assets = wall["items"].as_array().unwrap();
    let parent: photo_app_service::WallAsset = serde_json::from_value(
        assets
            .iter()
            .find(|asset| asset["displayName"] == "parent.jpg")
            .unwrap()
            .clone(),
    )
    .unwrap();
    let child: photo_app_service::WallAsset = serde_json::from_value(
        assets
            .iter()
            .find(|asset| asset["displayName"] == "child.jpg")
            .unwrap()
            .clone(),
    )
    .unwrap();
    let selection = gallery.resolve_selection(&id).unwrap();
    let base = gallery.current_event_id_for_test(&selection);
    let mut current = event_body(&app, &id, "current", "currentFolder", base).await;
    let mut broad = event_body(&app, &id, "broad", "includeSubfolders", base).await;
    assert_eq!(
        gallery.aggregate_scope_for_test(&selection).await,
        GalleryScope::IncludeSubfolders
    );
    gallery
        .publish_update_for_test(
            &selection,
            WallUpdate::CatalogBatch {
                selection_id: id.clone(),
                assets: vec![child.clone()],
                order_state: OrderState::Provisional,
                generation: 7,
                progress: Default::default(),
            },
        )
        .await;
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            next_sse_frame(&mut current)
        )
        .await
        .is_err()
    );
    assert!(next_sse_frame(&mut broad).await.contains("child.jpg"));
    gallery
        .publish_update_for_test(
            &selection,
            WallUpdate::CatalogBatch {
                selection_id: id.clone(),
                assets: vec![parent],
                order_state: OrderState::Provisional,
                generation: 8,
                progress: Default::default(),
            },
        )
        .await;
    assert!(next_sse_frame(&mut current).await.contains("parent.jpg"));
    assert!(next_sse_frame(&mut broad).await.contains("parent.jpg"));
    drop(broad);
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        gallery.aggregate_scope_for_test(&selection).await,
        GalleryScope::CurrentFolder
    );
    drop(current);
}

#[tokio::test(start_paused = true)]
async fn http_leases_aggregate_and_expire_after_thirty_seconds() {
    let (_temp, state, app) = test_state();
    let gallery = state.gallery_for_test().unwrap();
    let id = create_selection(&app, "").await;
    let selection = gallery.resolve_selection(&id).unwrap();
    let base = gallery.current_event_id_for_test(&selection);
    let current = event_body(&app, &id, "current", "currentFolder", base).await;
    let broad = event_body(&app, &id, "broad", "includeSubfolders", base).await;
    for _ in 0..4 {
        tokio::task::yield_now().await;
    }
    assert_eq!(gallery.scheduler_permits_for_test(), 1);
    let response = app
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/selections/{id}/interaction"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"clientId": "current", "scope": "currentFolder", "state": "idle"})
                        .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(gallery.scheduler_permits_for_test(), 1);
    tokio::time::advance(std::time::Duration::from_secs(31)).await;
    for _ in 0..6 {
        tokio::task::yield_now().await;
    }
    assert_eq!(gallery.scheduler_permits_for_test(), 4);
    assert_eq!(
        gallery.aggregate_scope_for_test(&selection).await,
        GalleryScope::IncludeSubfolders
    );
    drop(broad);
    for _ in 0..3 {
        tokio::task::yield_now().await;
    }
    assert_eq!(
        gallery.aggregate_scope_for_test(&selection).await,
        GalleryScope::CurrentFolder
    );
    drop(current);
}
