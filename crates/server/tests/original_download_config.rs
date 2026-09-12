use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use photo_server::{AppState, ConfigError, ServerConfig, build_router};
use std::ffi::OsString;
use std::sync::{Mutex, MutexGuard, OnceLock};
use tower::ServiceExt;

const ORIGINAL_DOWNLOADS: &str = "PHOTO_VIEWER_ALLOW_ORIGINAL_DOWNLOADS";
const CONFIG_ENVIRONMENT: [&str; 6] = [
    "PHOTO_VIEWER_DATA_DIR",
    "PHOTO_VIEWER_CACHE_DIR",
    "PHOTO_VIEWER_SOURCE_ROOT",
    "PHOTO_VIEWER_WEB_ROOT",
    "PHOTO_VIEWER_BIND",
    ORIGINAL_DOWNLOADS,
];

static ENVIRONMENT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

struct ConfigEnvironment {
    _lock: MutexGuard<'static, ()>,
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl ConfigEnvironment {
    fn new(temp: &tempfile::TempDir) -> Self {
        let lock = ENVIRONMENT_LOCK.get_or_init(|| Mutex::new(()));
        let lock = lock.lock().unwrap_or_else(|error| error.into_inner());
        let saved = CONFIG_ENVIRONMENT
            .into_iter()
            .map(|name| (name, std::env::var_os(name)))
            .collect();
        let source = temp.path().join("photos");
        let web = temp.path().join("web");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(&web).unwrap();
        unsafe {
            std::env::set_var("PHOTO_VIEWER_DATA_DIR", temp.path().join("data"));
            std::env::set_var("PHOTO_VIEWER_CACHE_DIR", temp.path().join("cache"));
            std::env::set_var("PHOTO_VIEWER_SOURCE_ROOT", source);
            std::env::set_var("PHOTO_VIEWER_WEB_ROOT", web);
            std::env::remove_var("PHOTO_VIEWER_BIND");
            std::env::remove_var(ORIGINAL_DOWNLOADS);
        }
        Self { _lock: lock, saved }
    }

    fn set_original_downloads(&self, value: Option<&str>) {
        unsafe {
            match value {
                Some(value) => std::env::set_var(ORIGINAL_DOWNLOADS, value),
                None => std::env::remove_var(ORIGINAL_DOWNLOADS),
            }
        }
    }
}

impl Drop for ConfigEnvironment {
    fn drop(&mut self) {
        unsafe {
            for (name, value) in &self.saved {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

#[tokio::test]
async fn bootstrap_disables_original_downloads_by_default() {
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
    let (state, _) = AppState::open(&config).unwrap();
    let app = build_router(state, config.static_web_root());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/bootstrap")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();

    assert_eq!(body["capabilities"]["originalDownloads"], false);
}

#[test]
fn original_download_environment_accepts_only_documented_boolean_values() {
    let temp = tempfile::tempdir().unwrap();
    let environment = ConfigEnvironment::new(&temp);

    for (value, expected) in [
        (None, false),
        (Some(""), false),
        (Some("0"), false),
        (Some("false"), false),
        (Some("FALSE"), false),
        (Some("FaLsE"), false),
        (Some("1"), true),
        (Some("true"), true),
        (Some("TRUE"), true),
        (Some("TrUe"), true),
    ] {
        environment.set_original_downloads(value);
        assert_eq!(
            ServerConfig::from_env().unwrap().allow_original_downloads(),
            expected,
            "{value:?}"
        );
    }

    for value in ["yes", "2", " true"] {
        environment.set_original_downloads(Some(value));
        assert!(matches!(
            ServerConfig::from_env(),
            Err(ConfigError::InvalidOriginalDownloads)
        ));
    }
}

#[tokio::test]
async fn bootstrap_advertises_original_downloads_when_enabled() {
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
    .unwrap()
    .with_allow_original_downloads(true);
    let (state, _) = AppState::open(&config).unwrap();
    let app = build_router(state, config.static_web_root());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/api/v1/bootstrap")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();

    assert_eq!(body["capabilities"]["originalDownloads"], true);
}
