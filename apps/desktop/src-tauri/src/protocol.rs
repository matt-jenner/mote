use photo_app_service::{AppService, DerivativeClass, WallUpdate};
use tauri::http::{Request, Response, StatusCode, Uri};

pub(crate) fn handle_derivative_request(
    service: &AppService,
    request: Request<Vec<u8>>,
) -> Response<Vec<u8>> {
    let parsed = parse_derivative_uri(request.uri());
    let (asset_id, class, key) = match parsed {
        Ok(value) => value,
        Err(status) => return empty_response(status),
    };
    match service.read_derivative(&asset_id, class, &key) {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "image/jpeg")
            .header("Cache-Control", "public, max-age=31536000, immutable")
            .header("X-Content-Type-Options", "nosniff")
            .body(bytes)
            .expect("derivative response headers are valid"),
        Err(photo_app_service::AppServiceError::InvalidAssetId)
        | Err(photo_app_service::AppServiceError::InvalidDerivativeKey) => {
            empty_response(StatusCode::BAD_REQUEST)
        }
        Err(_) => empty_response(StatusCode::NOT_FOUND),
    }
}

fn parse_derivative_uri(uri: &Uri) -> Result<(String, DerivativeClass, String), StatusCode> {
    let authority = uri.authority().map(|value| value.as_str());
    let approved_origin = matches!(
        (uri.scheme_str(), authority),
        (Some("photo-derivative"), Some("localhost"))
            | (Some("http"), Some("photo-derivative.localhost"))
    );
    if !approved_origin {
        return Err(StatusCode::BAD_REQUEST);
    }
    parse_derivative_path(uri.path())
}

fn parse_derivative_path(path: &str) -> Result<(String, DerivativeClass, String), StatusCode> {
    let segments = path
        .strip_prefix('/')
        .unwrap_or(path)
        .split('/')
        .collect::<Vec<_>>();
    if segments.len() != 3 || segments.iter().any(|segment| segment.is_empty()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    let class = match segments[1] {
        "wallThumbnail" => DerivativeClass::WallThumbnail,
        "screenPreview" => DerivativeClass::ScreenPreview,
        _ => return Err(StatusCode::BAD_REQUEST),
    };
    let key = segments[2];
    if key.len() > 256 || !key.is_ascii() || !key.bytes().all(|byte| byte.is_ascii_graphic()) {
        return Err(StatusCode::BAD_REQUEST);
    }
    Ok((segments[0].to_owned(), class, key.to_owned()))
}

fn empty_response(status: StatusCode) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .body(Vec::new())
        .expect("empty protocol response is valid")
}

pub(crate) async fn forward_wall_updates(
    receiver: tokio::sync::broadcast::Receiver<WallUpdate>,
    channel: tauri::ipc::Channel<WallUpdate>,
) {
    forward_wall_updates_inner(receiver, channel, |_| {}).await;
}

#[cfg(test)]
async fn forward_wall_updates_with_lag_hook<F>(
    receiver: tokio::sync::broadcast::Receiver<WallUpdate>,
    channel: tauri::ipc::Channel<WallUpdate>,
    on_lag: F,
) where
    F: FnMut(usize) + Send,
{
    forward_wall_updates_inner(receiver, channel, on_lag).await;
}

async fn forward_wall_updates_inner<F>(
    mut receiver: tokio::sync::broadcast::Receiver<WallUpdate>,
    channel: tauri::ipc::Channel<WallUpdate>,
    mut on_lag: F,
) where
    F: FnMut(usize) + Send,
{
    loop {
        match receiver.recv().await {
            Ok(update) => {
                if channel.send(update).is_err() {
                    return;
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                let retained_window = receiver.len();
                on_lag(retained_window);
                let mut newest_progress = None;
                let mut remaining = retained_window;
                while remaining > 0 {
                    match receiver.try_recv() {
                        Ok(update) => {
                            remaining = remaining.saturating_sub(1);
                            match update {
                                WallUpdate::CatalogBatch { progress, .. }
                                | WallUpdate::Progress { progress } => {
                                    newest_progress = Some(progress)
                                }
                                _ => {}
                            }
                        }
                        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
                        | Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                        Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                            let skipped = usize::try_from(skipped).unwrap_or(usize::MAX);
                            remaining = remaining.saturating_sub(skipped);
                        }
                    }
                }
                if let Some(progress) = newest_progress
                    && channel.send(WallUpdate::Progress { progress }).is_err()
                {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_app_service::{AppConfig, DerivativeReference, DerivativeRequest, SortDirection};
    use photo_app_service::{
        OrderState, ScanProgressDto, SourceAvailability, WallAsset, WallMediaKind, WallShapeState,
    };
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};

    static FIXTURE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TempFixture(PathBuf);

    impl Drop for TempFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct ProtocolFixture {
        _temp: TempFixture,
        pub service: AppService,
        pub source_root: PathBuf,
        asset_id: String,
        reference: DerivativeReference,
    }

    impl ProtocolFixture {
        fn with_ready_wall_thumbnail() -> Self {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async {
                    let temp = TempFixture(std::env::temp_dir().join(format!(
                        "photo-viewer-protocol-{}-{}",
                        std::process::id(),
                        FIXTURE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                    )));
                    std::fs::create_dir_all(&temp.0).unwrap();
                    let source = temp.0.join("photos");
                    std::fs::create_dir_all(&source).unwrap();
                    std::fs::write(
                        source.join("photo.jpg"),
                        include_bytes!("../../../../docs/superpowers/specs/assets/2026-08-24-macos-open-and-return-implementation.jpg"),
                    )
                    .unwrap();
                    let config = AppConfig::new(
                        temp.0.join("data"),
                        temp.0.join("cache"),
                    );
                    let service = AppService::open(config).unwrap();
                    let mut updates = service.subscribe_wall_updates();
                    service.start_scan(&source).await.unwrap();
                    while !matches!(updates.recv().await.unwrap(), WallUpdate::MetadataSettled { .. }) {}
                    let asset_id = service
                        .query_wall(photo_app_service::WallQueryRequest {
                            cursor: None,
                            limit: 1,
                            direction: SortDirection::OldestFirst,
                        })
                        .await
                        .unwrap()
                        .items[0]
                        .id
                        .clone();
                    service
                        .request_derivatives(DerivativeRequest::visible(vec![asset_id.clone()]))
                        .await
                        .unwrap();
                    let reference = loop {
                        if let WallUpdate::DerivativesReady { derivatives } = updates.recv().await.unwrap() {
                            break derivatives[0].clone();
                        }
                    };
                    Self {
                        _temp: temp,
                        service,
                        source_root: source,
                        asset_id,
                        reference,
                    }
                })
        }

        fn uri_with_kind(&self, kind: &str) -> String {
            format!(
                "photo-derivative://localhost/{}/{}/{}",
                self.asset_id, kind, self.reference.key
            )
        }

        fn uri_with_key(&self, key: &str) -> String {
            format!(
                "photo-derivative://localhost/{}/wallThumbnail/{}",
                self.asset_id, key
            )
        }

        fn uri_with_authority(&self, authority: &str) -> String {
            format!(
                "photo-derivative://{}/{}/{}/{}",
                authority, self.asset_id, "wallThumbnail", self.reference.key
            )
        }

        fn windows_uri(&self) -> String {
            format!(
                "http://photo-derivative.localhost/{}/{}/{}",
                self.asset_id, "wallThumbnail", self.reference.key
            )
        }

        fn uri_for_missing_asset(&self) -> String {
            format!(
                "photo-derivative://localhost/00000000-0000-0000-0000-000000000000/wallThumbnail/{}",
                self.reference.key
            )
        }
    }

    fn request(uri: &str) -> Request<Vec<u8>> {
        Request::builder().uri(uri).body(Vec::new()).unwrap()
    }

    #[test]
    fn derivative_protocol_rejects_malformed_or_mismatched_identity_without_paths() {
        let fixture = ProtocolFixture::with_ready_wall_thumbnail();
        let cases = vec![
            (
                "photo-derivative://localhost/not-a-uuid/wallThumbnail/key".to_owned(),
                400,
            ),
            (fixture.uri_with_kind("unknown"), 400),
            (fixture.uri_with_key(&"x".repeat(257)), 400),
            (fixture.uri_for_missing_asset(), 404),
            (fixture.uri_with_key("wrong-key"), 404),
            (fixture.uri_with_authority("unsupported.example"), 400),
        ];
        for (uri, expected) in cases {
            let response = handle_derivative_request(&fixture.service, request(&uri));
            assert_eq!(response.status().as_u16(), expected);
            assert!(
                !String::from_utf8_lossy(response.body())
                    .contains(fixture.source_root.to_string_lossy().as_ref())
            );
        }
    }

    #[test]
    fn derivative_protocol_returns_jpeg_with_cache_security_headers() {
        let fixture = ProtocolFixture::with_ready_wall_thumbnail();
        let response = handle_derivative_request(
            &fixture.service,
            request(&fixture.uri_with_authority("localhost")),
        );
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["Content-Type"], "image/jpeg");
        assert_eq!(
            response.headers()["Cache-Control"],
            "public, max-age=31536000, immutable"
        );
        assert_eq!(response.headers()["X-Content-Type-Options"], "nosniff");
        assert_eq!(&response.body()[..2], b"\xff\xd8");

        let windows_response =
            handle_derivative_request(&fixture.service, request(&fixture.windows_uri()));
        assert_eq!(windows_response.status(), StatusCode::OK);
    }

    fn sample_catalog_batch() -> WallUpdate {
        WallUpdate::CatalogBatch {
            assets: vec![WallAsset {
                id: "00000000-0000-0000-0000-000000000001".to_owned(),
                display_name: "sample.jpg".to_owned(),
                media_kind: WallMediaKind::Jpeg,
                provisional_order: 1,
                captured_at_utc: None,
                date_state: OrderState::Provisional,
                width: 16,
                height: 12,
                representative_rgb: None,
                shape_state: WallShapeState::Ready,
                availability: SourceAvailability::Available,
                warning: None,
                wall_thumbnail: None,
                screen_preview: None,
            }],
            order_state: OrderState::Provisional,
            progress: ScanProgressDto {
                discovered: 1,
                shaped: 1,
                enriched: 0,
                total: Some(1),
            },
        }
    }

    #[tokio::test]
    async fn wall_update_forwarder_sends_one_serializable_catalog_batch() {
        let (sender, receiver) = tokio::sync::broadcast::channel(4);
        let received = Arc::new(Mutex::new(Vec::<WallUpdate>::new()));
        let sink = received.clone();
        let channel = tauri::ipc::Channel::new(move |body| {
            sink.lock()
                .unwrap()
                .push(body.deserialize::<WallUpdate>().unwrap());
            Ok(())
        });
        let task = tokio::spawn(forward_wall_updates(receiver, channel));
        sender.send(sample_catalog_batch()).unwrap();
        drop(sender);
        task.await.unwrap();
        assert!(matches!(
            received.lock().unwrap().as_slice(),
            [WallUpdate::CatalogBatch { .. }]
        ));
    }

    fn progress(discovered: u64) -> WallUpdate {
        WallUpdate::Progress {
            progress: ScanProgressDto {
                discovered,
                shaped: discovered,
                enriched: discovered,
                total: Some(20),
            },
        }
    }

    fn later_source_update(source_id: &str) -> WallUpdate {
        WallUpdate::SourceUnavailable {
            source_id: source_id.to_owned(),
        }
    }

    #[tokio::test]
    async fn wall_update_forwarder_recovers_from_lag_and_resumes_ordered_live_updates() {
        let (sender, receiver) = tokio::sync::broadcast::channel(4);
        sender.send(later_source_update("retained-1")).unwrap();
        sender.send(progress(2)).unwrap();
        sender.send(later_source_update("retained-3")).unwrap();
        sender.send(later_source_update("retained-4")).unwrap();
        sender.send(later_source_update("retained-5")).unwrap();
        sender.send(progress(6)).unwrap();

        let received = Arc::new(Mutex::new(Vec::<WallUpdate>::new()));
        let sink = received.clone();
        let (snapshot_sent, snapshot_received) = tokio::sync::oneshot::channel();
        let first = Arc::new(Mutex::new(Some(snapshot_sent)));
        let first_signal = first.clone();
        let channel = tauri::ipc::Channel::new(move |body| {
            let update = body.deserialize::<WallUpdate>().unwrap();
            if matches!(update, WallUpdate::Progress { .. })
                && let Some(signal) = first_signal.lock().unwrap().take()
            {
                let _ = signal.send(());
            }
            sink.lock().unwrap().push(update);
            Ok(())
        });
        let mut later_sender = Some(sender.clone());
        let task = tokio::spawn(forward_wall_updates_with_lag_hook(
            receiver,
            channel,
            move |_| {
                let later_sender = later_sender.take().unwrap();
                later_sender.send(later_source_update("live-7")).unwrap();
                later_sender.send(later_source_update("live-8")).unwrap();
            },
        ));
        snapshot_received.await.unwrap();
        drop(sender);
        task.await.unwrap();

        let received = received.lock().unwrap();
        assert!(matches!(
            received.as_slice(),
            [
                WallUpdate::Progress { progress: ScanProgressDto { discovered: 6, .. } },
                WallUpdate::SourceUnavailable { source_id },
                WallUpdate::SourceUnavailable { source_id: second },
            ] if source_id == "live-7" && second == "live-8"
        ));
    }
}
