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
