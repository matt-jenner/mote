use std::path::Path;
use std::time::Duration;

use photo_app_service::{AppConfig, GalleryEngine, GalleryScope, WallUpdate};

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
