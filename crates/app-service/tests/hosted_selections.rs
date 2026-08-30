use std::path::Path;

use image::ImageBuffer;
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
    ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("real/photo.jpg"))
        .unwrap();
    std::os::unix::fs::symlink(source.join("real"), source.join("alias")).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();

    let real = engine.select_relative(Path::new("real")).await.unwrap();
    let alias = engine.select_relative(Path::new("alias")).await.unwrap();
    assert_eq!(real.id, alias.id);

    let alias_selection = engine.resolve_selection(&alias.id).unwrap();
    let mut updates = engine.subscribe(
        &alias_selection,
        "symlink-current-folder".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        None,
    );
    engine.ensure_running(&alias_selection).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            if matches!(
                updates.recv().await.unwrap().update,
                photo_app_service::WallUpdate::MetadataSettled { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("in-root symlink selection did not settle");
    let page = engine
        .query_wall(
            &alias_selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn retargeted_live_source_symlink_binds_only_the_new_canonical_library() {
    let temp = tempfile::tempdir().unwrap();
    let target_a = temp.path().join("photos-a");
    let target_b = temp.path().join("photos-b");
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&target_a).unwrap();
    std::fs::create_dir_all(&target_b).unwrap();
    std::fs::write(target_a.join("a.jpg"), b"a").unwrap();
    std::fs::write(target_b.join("b.jpg"), b"b").unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));

    let first = GalleryEngine::open(config.clone(), target_a.clone()).unwrap();
    let first_selection = first.select_relative(Path::new(".")).await.unwrap();
    drop(first);

    // A cataloged configured root may retain a user-facing alias in
    // display_path. That alias must not win over a live canonical retarget.
    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let library_id = photo_domain::LibraryId::from_uuid(
        uuid::Uuid::parse_str(&first_selection.source_id).unwrap(),
    );
    let canonical_a = std::fs::canonicalize(&target_a).unwrap();
    catalog
        .relink_library(library_id, &canonical_a, &source)
        .unwrap();

    std::os::unix::fs::symlink(&target_a, &source).unwrap();
    std::fs::remove_file(&source).unwrap();
    std::os::unix::fs::symlink(&target_b, &source).unwrap();

    let second = GalleryEngine::open(config, source).unwrap();
    let second_selection = second.select_relative(Path::new(".")).await.unwrap();
    assert_ne!(second_selection.id, first_selection.id);
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
    let mut events = engine.subscribe(
        &selection,
        "settlement".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        None,
    );
    engine.ensure_running(&selection).await.unwrap();
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(
            event.update,
            photo_app_service::WallUpdate::MetadataSettled { .. }
        ) {
            break;
        }
    }
    drop(events);
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
    let mut offline_events = reopened.subscribe(
        &selection,
        "offline".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        None,
    );
    reopened.ensure_running(&selection).await.unwrap();
    reopened.ensure_running(&selection).await.unwrap();
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), offline_events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event.update,
        photo_app_service::WallUpdate::SourceUnavailable { .. }
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), offline_events.recv())
            .await
            .is_err()
    );
    assert_eq!(
        reopened.selection_summary(&selection).unwrap().availability,
        photo_app_service::SourceAvailability::RootOffline
    );
}

#[tokio::test]
async fn hosted_derivative_requests_enforce_current_folder_and_foreign_membership() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("Parent/Child")).unwrap();
    ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("Parent/root.jpg"))
        .unwrap();
    ImageBuffer::from_pixel(3, 2, image::Rgb([80_u8, 180_u8, 220_u8]))
        .save(source.join("Parent/Child/child.jpg"))
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new("Parent")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    engine.ensure_running(&selection).await.unwrap();
    let page = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let page = engine
                .query_wall(
                    &selection,
                    photo_app_service::GalleryScope::IncludeSubfolders,
                    photo_app_service::WallQueryRequest::oldest_first(),
                )
                .await
                .unwrap();
            if page.items.len() == 2 {
                break page;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let child_id = page
        .items
        .iter()
        .find(|item| item.display_name.contains("child"))
        .unwrap()
        .id
        .clone();
    assert!(matches!(
        engine.validate_derivative_request(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            &photo_app_service::DerivativeRequest::visible(vec![child_id.clone()]),
        ),
        Err(photo_app_service::AppServiceError::ForeignAsset)
    ));
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::IncludeSubfolders,
            photo_app_service::DerivativeRequest::visible(vec![child_id]),
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn hosted_selection_can_request_and_open_a_managed_thumbnail() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    image::ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("photo.jpg"))
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    engine.ensure_running(&selection).await.unwrap();
    let page = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let page = engine
                .query_wall(
                    &selection,
                    photo_app_service::GalleryScope::CurrentFolder,
                    photo_app_service::WallQueryRequest::oldest_first(),
                )
                .await
                .unwrap();
            if !page.items.is_empty() {
                break page;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let asset_id = page.items[0].id.clone();
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();
    let asset_id = page.items[0].id.clone();
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible_screen_preview(vec![asset_id]),
        )
        .await
        .unwrap();
    let page = engine
        .query_wall(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    let key = page.items[0].screen_preview.as_ref().unwrap().key.clone();
    let mut managed = engine.open_derivative(&key).unwrap();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut managed.file, &mut bytes).unwrap();
    assert_eq!(&bytes[..3], &[0xff, 0xd8, 0xff]);
}

#[tokio::test]
async fn corrupt_hosted_thumbnail_is_repaired_before_it_is_reused() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    image::ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("photo.jpg"))
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    engine.ensure_running(&selection).await.unwrap();
    let page = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let page = engine
                .query_wall(
                    &selection,
                    photo_app_service::GalleryScope::CurrentFolder,
                    photo_app_service::WallQueryRequest::oldest_first(),
                )
                .await
                .unwrap();
            if !page.items.is_empty() {
                break page;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let _events = engine.subscribe(
        &selection,
        "repair-retained-runtime".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        None,
    );
    let asset_id = page.items[0].id.clone();
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();
    let key = engine
        .query_wall(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap()
        .items[0]
        .wall_thumbnail
        .as_ref()
        .unwrap()
        .key
        .clone();
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let record = catalog.find_derivative_by_cache_key(&key).unwrap().unwrap();
    let cache_path = config.cache_dir().join(record.relative_cache_path);
    std::fs::write(&cache_path, b"corrupt").unwrap();
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();
    let repaired = std::fs::read(cache_path).unwrap();
    assert_eq!(&repaired[..3], &[0xff, 0xd8, 0xff]);
    let attempts_before = engine.hosted_derivative_attempts_for_test();

    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let record = catalog.find_derivative_by_cache_key(&key).unwrap().unwrap();
    let cache_path = config.cache_dir().join(record.relative_cache_path);
    std::fs::write(&cache_path, [0xff_u8, 0xd8, 0xff]).unwrap();
    let first = engine.clone();
    let second = engine.clone();
    let first_selection = selection.clone();
    let second_selection = selection.clone();
    let (first, second) = tokio::join!(
        first.request_derivatives(
            &first_selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        ),
        second.request_derivatives(
            &second_selection,
            photo_app_service::GalleryScope::IncludeSubfolders,
            photo_app_service::DerivativeRequest::visible(vec![asset_id]),
        )
    );
    first.unwrap();
    second.unwrap();
    let repaired = std::fs::read(cache_path).unwrap();
    assert!(repaired.len() > 3);
    assert_eq!(&repaired[..3], &[0xff, 0xd8, 0xff]);
    assert_eq!(
        engine.hosted_derivative_attempts_for_test(),
        attempts_before + 1
    );
}
