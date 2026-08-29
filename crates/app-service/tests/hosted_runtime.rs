use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use photo_app_service::{
    AppConfig, DerivativeClass, DerivativeReference, GalleryEngine, GalleryScope, InteractionState,
    MetadataReader, WallUpdate, WallWarningState,
};
use photo_metadata::{MetadataBundle, MetadataReadWarning};

#[tokio::test]
async fn separate_selection_runtimes_publish_only_their_selection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("Parent/Child")).unwrap();
    write_jpeg(&source.join("Parent/a.jpg"));
    write_jpeg(&source.join("Parent/Child/b.jpg"));
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let parent = engine.select_relative(Path::new("Parent")).await.unwrap();
    let child = engine
        .select_relative(Path::new("Parent/Child"))
        .await
        .unwrap();
    let mut parent_events = engine.subscribe(
        &engine.resolve_selection(&parent.id).unwrap(),
        "parent".into(),
        GalleryScope::IncludeSubfolders,
        None,
    );
    let mut child_events = engine.subscribe(
        &engine.resolve_selection(&child.id).unwrap(),
        "child".into(),
        GalleryScope::CurrentFolder,
        None,
    );
    let parent_selection = engine.resolve_selection(&parent.id).unwrap();
    let child_selection = engine.resolve_selection(&child.id).unwrap();
    let (parent_result, child_result) = tokio::join!(
        engine.ensure_running(&parent_selection),
        engine.ensure_running(&child_selection)
    );
    parent_result.unwrap();
    child_result.unwrap();

    let parent_id = parent.id.clone();
    let child_id = child.id.clone();
    let parent_seen = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(event) = parent_events.recv().await {
                if matches!(event.update, WallUpdate::CatalogBatch { .. }) {
                    break event;
                }
            }
        }
    })
    .await
    .unwrap();
    let child_seen = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if let Some(event) = child_events.recv().await {
                if matches!(event.update, WallUpdate::CatalogBatch { .. }) {
                    break event;
                }
            }
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(parent_seen.update, WallUpdate::CatalogBatch { ref selection_id, .. } if selection_id == &parent_id)
    );
    assert!(
        matches!(child_seen.update, WallUpdate::CatalogBatch { ref selection_id, .. } if selection_id == &child_id)
    );
}

fn write_jpeg(path: &Path) {
    let image = image::ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]));
    image.save(path).unwrap();
}

#[tokio::test]
async fn an_old_subscription_drop_cannot_remove_a_reconnected_client_demand() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    let old = engine.subscribe(
        &selection,
        "same-client".into(),
        GalleryScope::CurrentFolder,
        None,
    );
    let replacement = engine.subscribe(
        &selection,
        "same-client".into(),
        GalleryScope::IncludeSubfolders,
        None,
    );
    drop(old);
    assert_eq!(
        engine.aggregate_scope_for_test(&selection).await,
        GalleryScope::IncludeSubfolders
    );
    drop(replacement);
}

#[tokio::test]
async fn a_reconnected_subscription_drop_cannot_remove_the_original_client_demand() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    let original = engine.subscribe(
        &selection,
        "same-client".to_owned(),
        GalleryScope::IncludeSubfolders,
        None,
    );
    let replacement = engine.subscribe(
        &selection,
        "same-client".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    drop(replacement);
    assert_eq!(
        engine.aggregate_scope_for_test(&selection).await,
        GalleryScope::IncludeSubfolders
    );
    drop(original);
}

#[derive(Clone, Default)]
struct CountingReader(Arc<AtomicUsize>);

impl MetadataReader for CountingReader {
    fn read(
        &self,
        _media_path: &Path,
        _sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(MetadataBundle::default())
    }
}

#[derive(Clone)]
struct BlockingReader {
    entered: Arc<tokio::sync::Notify>,
    gate: Arc<(Mutex<bool>, Condvar)>,
    starts: Arc<AtomicUsize>,
}

impl BlockingReader {
    fn new() -> Self {
        Self {
            entered: Arc::new(tokio::sync::Notify::new()),
            gate: Arc::new((Mutex::new(false), Condvar::new())),
            starts: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn release(&self) {
        let (released, changed) = &*self.gate;
        *released.lock().unwrap() = true;
        changed.notify_all();
    }
}

impl MetadataReader for BlockingReader {
    fn read(
        &self,
        media_path: &Path,
        sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        self.starts.fetch_add(1, Ordering::SeqCst);
        self.entered.notify_one();
        let (released, changed) = &*self.gate;
        let mut released = released.lock().unwrap();
        while !*released {
            released = changed.wait(released).unwrap();
        }
        photo_indexer::DefaultMetadataReader.read(media_path, sidecar_path)
    }
}

#[tokio::test]
async fn same_selection_shares_one_scan_and_different_selections_start_two() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("left")).unwrap();
    std::fs::create_dir_all(source.join("right")).unwrap();
    write_jpeg(&source.join("left/photo.jpg"));
    write_jpeg(&source.join("right/photo.jpg"));
    let reader = CountingReader::default();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open_with_reader(config, source, Arc::new(reader.clone())).unwrap();
    let left = engine.select_relative(Path::new("left")).await.unwrap();
    let left_selection = engine.resolve_selection(&left.id).unwrap();
    let mut left_events = engine.subscribe(
        &left_selection,
        "left".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );

    let (first, second) = tokio::join!(
        engine.ensure_running(&left_selection),
        engine.ensure_running(&left_selection)
    );
    first.unwrap();
    second.unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = left_events.recv().await.unwrap();
            if matches!(event.update, WallUpdate::MetadataSettled { .. }) {
                break;
            }
        }
    })
    .await
    .expect("same-selection scan did not settle");
    assert_eq!(reader.0.load(Ordering::SeqCst), 1);

    let right = engine.select_relative(Path::new("right")).await.unwrap();
    let right_selection = engine.resolve_selection(&right.id).unwrap();
    let mut right_events = engine.subscribe(
        &right_selection,
        "right".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    engine.ensure_running(&right_selection).await.unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = right_events.recv().await.unwrap();
            if matches!(event.update, WallUpdate::MetadataSettled { .. }) {
                break;
            }
        }
    })
    .await
    .expect("different-selection scan did not settle");
    assert_eq!(reader.0.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn different_selection_scans_share_the_scheduler_admission_cap() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("left")).unwrap();
    std::fs::create_dir_all(source.join("right")).unwrap();
    write_jpeg(&source.join("left/photo.jpg"));
    write_jpeg(&source.join("right/photo.jpg"));
    let reader = BlockingReader::new();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open_with_reader(config, source, Arc::new(reader.clone())).unwrap();
    engine
        .set_interaction_for_test(InteractionState::Active)
        .await;
    let left = engine.select_relative(Path::new("left")).await.unwrap();
    let right = engine.select_relative(Path::new("right")).await.unwrap();
    let left_selection = engine.resolve_selection(&left.id).unwrap();
    let right_selection = engine.resolve_selection(&right.id).unwrap();
    let mut left_events = engine.subscribe(
        &left_selection,
        "left-cap".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    let mut right_events = engine.subscribe(
        &right_selection,
        "right-cap".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    let entered_wait = reader.entered.notified();
    tokio::pin!(entered_wait);
    entered_wait.as_mut().enable();
    let (left_result, right_result) = tokio::join!(
        engine.ensure_running(&left_selection),
        engine.ensure_running(&right_selection)
    );
    left_result.unwrap();
    right_result.unwrap();
    tokio::time::timeout(Duration::from_secs(15), entered_wait)
        .await
        .expect("a cross-selection scan did not reach metadata");
    assert_eq!(reader.starts.load(Ordering::SeqCst), 1);
    reader.release();
    for subscription in [&mut left_events, &mut right_events] {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                if matches!(
                    subscription.recv().await.unwrap().update,
                    WallUpdate::MetadataSettled { .. }
                ) {
                    break;
                }
            }
        })
        .await
        .expect("cross-selection scan did not settle");
    }
    assert_eq!(reader.starts.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn primary_scan_survives_last_stream_drop() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    write_jpeg(&source.join("photo.jpg"));
    let reader = BlockingReader::new();
    let entered = reader.entered.clone();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine =
        GalleryEngine::open_with_reader(config.clone(), source, Arc::new(reader.clone())).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    let stream = engine.subscribe(
        &selection,
        "primary".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    let entered_wait = entered.notified();
    tokio::pin!(entered_wait);
    entered_wait.as_mut().enable();
    engine.ensure_running(&selection).await.unwrap();
    tokio::time::timeout(Duration::from_secs(15), entered_wait)
        .await
        .expect("scan did not reach metadata reader");
    drop(stream);

    let mut replacement = engine.subscribe(
        &selection,
        "replacement".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    reader.release();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let event = replacement.recv().await.unwrap();
            if matches!(event.update, WallUpdate::MetadataSettled { .. }) {
                break;
            }
        }
    })
    .await
    .expect("primary scan did not survive stream drop");
    let page = engine
        .query_wall(
            &selection,
            GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
}

#[tokio::test]
async fn mixed_scope_filters_catalog_derivatives_and_warning_events() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("child")).unwrap();
    write_jpeg(&source.join("root.jpg"));
    write_jpeg(&source.join("child/child.jpg"));
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    let mut current = engine.subscribe(
        &selection,
        "current".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    let mut recursive = engine.subscribe(
        &selection,
        "recursive".to_owned(),
        GalleryScope::IncludeSubfolders,
        None,
    );
    engine.ensure_running(&selection).await.unwrap();
    let current_catalog = tokio::time::timeout(Duration::from_secs(15), async {
        let mut names = Vec::new();
        loop {
            let event = current.recv().await.unwrap();
            match event.update {
                WallUpdate::CatalogBatch { assets, .. } => {
                    names.extend(assets.into_iter().map(|asset| asset.display_name));
                }
                WallUpdate::MetadataSettled { .. } => break names,
                _ => {}
            }
        }
    })
    .await
    .expect("current-folder mixed-scope scan did not settle");
    let recursive_catalog = tokio::time::timeout(Duration::from_secs(15), async {
        let mut names = Vec::new();
        loop {
            let event = recursive.recv().await.unwrap();
            match event.update {
                WallUpdate::CatalogBatch { assets, .. } => {
                    names.extend(assets.into_iter().map(|asset| asset.display_name));
                }
                WallUpdate::MetadataSettled { .. } => break names,
                _ => {}
            }
        }
    })
    .await
    .expect("recursive mixed-scope scan did not settle");
    assert_eq!(current_catalog, ["root.jpg"]);
    assert_eq!(
        recursive_catalog
            .into_iter()
            .collect::<std::collections::HashSet<_>>(),
        ["root.jpg".to_owned(), "child.jpg".to_owned()]
            .into_iter()
            .collect()
    );

    let page = engine
        .query_wall(
            &selection,
            GalleryScope::IncludeSubfolders,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    let root_id = page
        .items
        .iter()
        .find(|asset| asset.display_name == "root.jpg")
        .unwrap()
        .id
        .clone();
    let child_id = page
        .items
        .iter()
        .find(|asset| asset.display_name == "child.jpg")
        .unwrap()
        .id
        .clone();
    let source_id = summary.source_id;
    engine
        .publish_update_for_test(
            &selection,
            WallUpdate::DerivativesReady {
                selection_id: summary.id.clone(),
                derivatives: vec![
                    DerivativeReference {
                        asset_id: root_id.clone(),
                        kind: DerivativeClass::WallThumbnail,
                        key: "root-thumb".to_owned(),
                    },
                    DerivativeReference {
                        asset_id: child_id.clone(),
                        kind: DerivativeClass::WallThumbnail,
                        key: "child-thumb".to_owned(),
                    },
                ],
            },
        )
        .await;
    let current_event = current.recv().await.unwrap();
    let recursive_event = recursive.recv().await.unwrap();
    assert!(
        matches!(current_event.update, WallUpdate::DerivativesReady { ref derivatives, .. } if derivatives.len() == 1 && derivatives[0].asset_id == root_id)
    );
    assert!(
        matches!(recursive_event.update, WallUpdate::DerivativesReady { ref derivatives, .. } if derivatives.len() == 2)
    );

    for asset_id in [root_id.clone(), child_id.clone()] {
        engine
            .publish_update_for_test(
                &selection,
                WallUpdate::Warning {
                    selection_id: summary.id.clone(),
                    source_id: source_id.clone(),
                    asset_id: Some(asset_id.clone()),
                    warning: WallWarningState {
                        code: "test-warning".to_owned(),
                        retryable: true,
                    },
                },
            )
            .await;
        if asset_id == root_id {
            assert!(
                matches!(current.recv().await.unwrap().update, WallUpdate::Warning { ref asset_id, .. } if asset_id.as_deref() == Some(root_id.as_str()))
            );
            assert!(
                matches!(recursive.recv().await.unwrap().update, WallUpdate::Warning { ref asset_id, .. } if asset_id.as_deref() == Some(root_id.as_str()))
            );
        } else {
            assert!(
                tokio::time::timeout(Duration::from_millis(50), current.recv())
                    .await
                    .is_err()
            );
            assert!(
                matches!(recursive.recv().await.unwrap().update, WallUpdate::Warning { ref asset_id, .. } if asset_id.as_deref() == Some(child_id.as_str()))
            );
        }
    }

    for asset_id in [root_id.clone(), child_id.clone()] {
        engine
            .publish_update_for_test(
                &selection,
                WallUpdate::WarningCleared {
                    selection_id: summary.id.clone(),
                    source_id: source_id.clone(),
                    asset_id: Some(asset_id.clone()),
                    code: "test-warning".to_owned(),
                },
            )
            .await;
        if asset_id == root_id {
            assert!(matches!(
                current.recv().await.unwrap().update,
                WallUpdate::WarningCleared { .. }
            ));
            assert!(matches!(
                recursive.recv().await.unwrap().update,
                WallUpdate::WarningCleared { .. }
            ));
        } else {
            assert!(
                tokio::time::timeout(Duration::from_millis(50), current.recv())
                    .await
                    .is_err()
            );
            assert!(matches!(
                recursive.recv().await.unwrap().update,
                WallUpdate::WarningCleared { .. }
            ));
        }
    }
}

fn progress(selection_id: &str, generation: u64) -> WallUpdate {
    WallUpdate::Progress {
        selection_id: selection_id.to_owned(),
        generation,
        progress: Default::default(),
    }
}

#[tokio::test]
async fn replay_retained_edges_and_live_boundary_are_serialized() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    let seed = engine.subscribe(
        &selection,
        "seed".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );

    for generation in 0..256 {
        engine
            .publish_update_for_test(&selection, progress(selection.id(), generation))
            .await;
    }
    let mut boundary = engine.subscribe(
        &selection,
        "boundary".to_owned(),
        GalleryScope::CurrentFolder,
        Some(1),
    );
    drop(seed);
    engine
        .publish_update_for_test(&selection, progress(selection.id(), 256))
        .await;
    let mut ids = Vec::new();
    while ids.last().copied() != Some(257) {
        ids.push(boundary.recv().await.unwrap().id);
    }
    assert_eq!(ids, (2..=257).collect::<Vec<_>>());

    let mut too_old = engine.subscribe(
        &selection,
        "too-old".to_owned(),
        GalleryScope::CurrentFolder,
        Some(0),
    );
    let old = too_old.recv().await.unwrap();
    assert_eq!(old.id, 257);
    assert!(matches!(old.update, WallUpdate::ResyncRequired { .. }));

    let mut too_new = engine.subscribe(
        &selection,
        "too-new".to_owned(),
        GalleryScope::CurrentFolder,
        Some(300),
    );
    let new = too_new.recv().await.unwrap();
    assert_eq!(new.id, 300);
    assert!(matches!(new.update, WallUpdate::ResyncRequired { .. }));
}

#[tokio::test]
async fn broadcast_lag_resync_watermark_is_monotonic() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    let seed = engine.subscribe(
        &selection,
        "seed".to_owned(),
        GalleryScope::CurrentFolder,
        None,
    );
    for generation in 0..32 {
        engine
            .publish_update_for_test(&selection, progress(selection.id(), generation))
            .await;
    }
    let mut lagged = engine.subscribe(
        &selection,
        "lagged".to_owned(),
        GalleryScope::CurrentFolder,
        Some(32),
    );
    drop(seed);
    for generation in 32..332 {
        engine
            .publish_update_for_test(&selection, progress(selection.id(), generation))
            .await;
    }
    let resync = lagged.recv().await.unwrap();
    assert_eq!(resync.id, 332);
    assert!(matches!(resync.update, WallUpdate::ResyncRequired { .. }));
    engine
        .publish_update_for_test(&selection, progress(selection.id(), 332))
        .await;
    let live = lagged.recv().await.unwrap();
    assert_eq!(live.id, 333);
    assert!(live.id > resync.id);
}
