use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use photo_server::{AppState, ServerConfig, StaticWebRoot, build_router};
use tower::ServiceExt;

#[tokio::test]
async fn source_disabled_startup_keeps_health_and_api_available_without_static_serving() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web = temp.path().join("web");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&web).unwrap();
    std::fs::write(web.join("index.html"), b"source sentinel").unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        web,
    )
    .unwrap();
    std::fs::create_dir_all(config.cache_dir()).unwrap();
    let partial = config.cache_dir().join("orphan.partial-source-disabled");
    std::fs::write(&partial, b"incomplete cache write").unwrap();

    let (state, report) = AppState::open_source_disabled_for_test(&config).unwrap();
    assert_eq!(report.partial_files_removed, 1);
    assert_eq!(report.missing_rows_removed, 0);
    assert!(!partial.exists());
    assert!(config.data_dir().is_dir());
    assert!(config.cache_dir().is_dir());
    let app = build_router(
        state,
        StaticWebRoot::unavailable_for_test(config.web_root().to_owned()),
    );

    let health = request(&app, "/healthz").await;
    assert_eq!(health.status(), StatusCode::OK);

    let bootstrap = json(request(&app, "/api/v1/bootstrap").await).await;
    assert_eq!(bootstrap["capabilities"]["folderBrowser"], false);
    assert_eq!(bootstrap["sourceAvailable"], false);
    assert!(bootstrap.as_object().unwrap().contains_key("accentColor"));
    assert_eq!(bootstrap["accentColor"], serde_json::Value::Null);

    for uri in [
        "/api/v1/folders",
        "/api/v1/selections/not-present",
        "/api/v1/selections/not-present/wall?scope=includeSubfolders&direction=newestFirst&limit=10",
    ] {
        let response = request(&app, uri).await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE, "{uri}");
        let body = json(response).await;
        assert_eq!(body["code"], "sourceUnavailable", "{uri}");
        assert_eq!(body["message"], "The photo source is unavailable.", "{uri}");
        assert!(
            !body
                .to_string()
                .contains(temp.path().to_string_lossy().as_ref())
        );
    }

    let static_response = request(&app, "/").await;
    assert_eq!(static_response.status(), StatusCode::NOT_FOUND);
    assert!(
        static_response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .is_empty()
    );
}

#[cfg(not(unix))]
#[test]
fn production_non_unix_startup_uses_the_source_disabled_composer() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let web = temp.path().join("web");
    std::fs::create_dir(&source).unwrap();
    std::fs::create_dir(&web).unwrap();
    let config = ServerConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
        None,
        source,
        web,
    )
    .unwrap();

    assert!(AppState::open(&config).is_ok());
}

async fn request(app: &axum::Router, uri: &str) -> axum::response::Response {
    app.clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap()
}
