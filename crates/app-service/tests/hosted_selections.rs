use std::path::Path;

use photo_app_service::{AppConfig, GalleryEngine};

#[tokio::test]
async fn selection_ids_are_stable_across_reopen_and_are_not_paths() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("Parent/Child")).unwrap();
    std::fs::write(source.join("Parent/a.jpg"), b"not an image").unwrap();
    std::fs::write(source.join("Parent/Child/b.jpg"), b"not an image").unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));

    let engine = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    let parent = engine.select_relative(Path::new("Parent")).await.unwrap();
    let child = engine
        .select_relative(Path::new("Parent/Child"))
        .await
        .unwrap();
    assert_ne!(parent.id, child.id);
    assert!(parent.id.starts_with("selection-"));
    assert!(
        !serde_json::to_string(&parent)
            .unwrap()
            .contains(&source.to_string_lossy().to_string())
    );

    let parent_id = parent.id.clone();
    let child_id = child.id.clone();
    drop(engine);
    let reopened = GalleryEngine::open(config, source).unwrap();
    assert_eq!(
        reopened.resolve_selection(&parent_id).unwrap().id(),
        parent_id
    );
    assert_eq!(
        reopened.resolve_selection(&child_id).unwrap().id(),
        child_id
    );
}

#[tokio::test]
async fn selection_id_aliases_are_rejected_before_runtime_creation() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let uuid = summary.id.strip_prefix("selection-").unwrap();
    let uppercase = format!("selection-{}", uuid.to_uppercase());
    assert!(engine.resolve_selection(&uppercase).is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn selection_rejects_a_symlink_that_escapes_the_hosted_root() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::os::unix::fs::symlink(&outside, source.join("escape")).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    assert!(engine.select_relative(Path::new("escape")).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn in_root_symlink_and_real_path_share_selection_identity() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("real")).unwrap();
    std::fs::write(source.join("real/photo.jpg"), b"not an image").unwrap();
    std::os::unix::fs::symlink(source.join("real"), source.join("alias")).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();

    let real = engine.select_relative(Path::new("real")).await.unwrap();
    let alias = engine.select_relative(Path::new("alias")).await.unwrap();
    assert_eq!(real.id, alias.id);
}

#[tokio::test]
async fn completed_selection_reopens_for_cached_browsing_when_source_is_offline() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    let image = image::ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]));
    image.save(source.join("photo.jpg")).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));

    let engine = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    engine.ensure_running(&selection).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    drop(engine);

    let offline = temp.path().join("offline-photos");
    std::fs::rename(&source, &offline).unwrap();
    let reopened = GalleryEngine::open(config, source).unwrap();
    let selection = reopened.resolve_selection(&summary.id).unwrap();
    let page = reopened
        .query_wall(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    reopened.ensure_running(&selection).await.unwrap();
}
