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
    let unavailable = reopened
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![page.items[0].id.clone()]),
        )
        .await
        .expect_err("offline generation without cache must be unavailable");
    assert!(matches!(
        unavailable,
        photo_app_service::AppServiceError::DerivativeUnavailable
    ));
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
    let root_id = page
        .items
        .iter()
        .find(|item| item.display_name.contains("root"))
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
    let child_selection_summary = engine
        .select_relative(Path::new("Parent/Child"))
        .await
        .unwrap();
    let child_selection = engine
        .resolve_selection(&child_selection_summary.id)
        .unwrap();
    assert!(matches!(
        engine.validate_derivative_request(
            &child_selection,
            photo_app_service::GalleryScope::CurrentFolder,
            &photo_app_service::DerivativeRequest::visible(vec![root_id]),
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
async fn hosted_derivative_requests_reject_an_asset_from_a_foreign_library() {
    let temp = tempfile::tempdir().unwrap();
    let first_source = temp.path().join("photos-a");
    let second_source = temp.path().join("photos-b");
    std::fs::create_dir_all(&first_source).unwrap();
    std::fs::create_dir_all(&second_source).unwrap();
    ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(first_source.join("first.jpg"))
        .unwrap();
    ImageBuffer::from_pixel(3, 2, image::Rgb([80_u8, 180_u8, 220_u8]))
        .save(second_source.join("second.jpg"))
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let first_engine = GalleryEngine::open(config.clone(), first_source).unwrap();
    let first_summary = first_engine.select_relative(Path::new(".")).await.unwrap();
    let first_selection = first_engine.resolve_selection(&first_summary.id).unwrap();
    first_engine.ensure_running(&first_selection).await.unwrap();
    let second_engine = GalleryEngine::open(config, second_source).unwrap();
    let second_summary = second_engine.select_relative(Path::new(".")).await.unwrap();
    let second_selection = second_engine.resolve_selection(&second_summary.id).unwrap();
    second_engine
        .ensure_running(&second_selection)
        .await
        .unwrap();
    let foreign_id = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let page = second_engine
                .query_wall(
                    &second_selection,
                    photo_app_service::GalleryScope::CurrentFolder,
                    photo_app_service::WallQueryRequest::oldest_first(),
                )
                .await
                .unwrap();
            if let Some(item) = page.items.first() {
                break item.id.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(matches!(
        first_engine.validate_derivative_request(
            &first_selection,
            photo_app_service::GalleryScope::CurrentFolder,
            &photo_app_service::DerivativeRequest::visible(vec![foreign_id]),
        ),
        Err(photo_app_service::AppServiceError::ForeignAsset)
    ));
}

#[tokio::test]
async fn hosted_derivative_service_rejects_empty_and_oversized_id_lists() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    for asset_ids in [Vec::new(), vec!["not-an-id".to_owned(); 251]] {
        let error = engine
            .validate_derivative_request(
                &selection,
                photo_app_service::GalleryScope::CurrentFolder,
                &photo_app_service::DerivativeRequest::visible(asset_ids),
            )
            .expect_err("hosted service must reject this list size");
        assert!(matches!(
            error,
            photo_app_service::AppServiceError::InvalidLimit
        ));
    }
}

#[tokio::test]
async fn hosted_selection_can_request_and_open_a_managed_thumbnail() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    image::ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("photo.jpg"))
        .unwrap();
    let source_file = source.join("photo.jpg");
    let source_before = std::fs::read(&source_file).unwrap();
    let source_mtime_before = std::fs::metadata(&source_file).unwrap().modified().unwrap();
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
    let source_after = std::fs::read(&source_file).unwrap();
    assert_eq!(source_after, source_before);
    assert_eq!(source_after.len(), source_before.len());
    assert_eq!(
        std::fs::metadata(&source_file).unwrap().modified().unwrap(),
        source_mtime_before
    );
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
    std::fs::remove_file(&cache_path).unwrap();
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();
    assert!(image::load_from_memory(&std::fs::read(&cache_path).unwrap()).is_ok());
    std::fs::write(&cache_path, b"corrupt").unwrap();
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();
    let repaired = std::fs::read(&cache_path).unwrap();
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
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
    );
    first.unwrap();
    second.unwrap();
    let repaired = std::fs::read(&cache_path).unwrap();
    assert!(repaired.len() > 3);
    assert_eq!(&repaired[..3], &[0xff, 0xd8, 0xff]);
    assert_eq!(
        engine.hosted_derivative_attempts_for_test(),
        attempts_before + 1
    );

    // A corrupt file can retain the expected length and JPEG prefix. It must
    // still be decoded before an immutable cache key is treated as reusable.
    let valid = std::fs::read(&cache_path).unwrap();
    let mut same_size_corrupt = vec![0xff_u8, 0xd8, 0xff];
    same_size_corrupt.resize(valid.len(), 0);
    std::fs::write(&cache_path, same_size_corrupt).unwrap();
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![page.items[0].id.clone()]),
        )
        .await
        .unwrap();
    let repaired = std::fs::read(cache_path).unwrap();
    assert!(image::load_from_memory(&repaired).is_ok());

    // A mutation arriving after bytes are staged but before the final
    // publication decision must remove this requester's link and leave no
    // reusable stale catalog row.
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_commit_post_publication_test_gate(entered.clone(), release.clone())
        .await;
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let record = catalog.find_derivative_by_cache_key(&key).unwrap().unwrap();
    std::fs::write(
        config.cache_dir().join(&record.relative_cache_path),
        b"corrupt",
    )
    .unwrap();
    let request_engine = engine.clone();
    let request_selection = selection.clone();
    let request_asset = page.items[0].id.clone();
    let request = tokio::spawn(async move {
        request_engine
            .request_derivatives(
                &request_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![request_asset]),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .expect("repair did not reach the post-publication barrier");
    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let asset_id_value =
        photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&asset_id).unwrap());
    let existing = catalog.find_asset(asset_id_value).unwrap().unwrap();
    catalog
        .upsert_asset(&photo_catalog::NewAsset {
            id: existing.id,
            library_id: existing.library_id,
            relative_path: existing.relative_path.clone(),
            display_path: existing.display_path.clone(),
            media_kind: existing.media_kind,
            signature: photo_domain::FileSignature {
                modified_unix_ns: existing.signature.modified_unix_ns.wrapping_add(1),
                ..existing.signature
            },
            folder_group_id: existing.folder_group_id,
        })
        .unwrap();
    release.notify_waiters();
    assert!(matches!(
        request.await.unwrap().unwrap_err(),
        photo_app_service::AppServiceError::DerivativeUnavailable
    ));
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert!(
        catalog
            .find_derivative_by_cache_key(&key)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn hosted_repair_reuses_a_shared_derivative_linked_to_another_group() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(source.join("child")).unwrap();
    ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("child/photo.jpg"))
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source).unwrap();
    let root_summary = engine.select_relative(Path::new(".")).await.unwrap();
    let child_summary = engine.select_relative(Path::new("child")).await.unwrap();
    let root = engine.resolve_selection(&root_summary.id).unwrap();
    let child = engine.resolve_selection(&child_summary.id).unwrap();
    engine.ensure_running(&root).await.unwrap();
    engine.ensure_running(&child).await.unwrap();
    let root_page = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let page = engine
                .query_wall(
                    &root,
                    photo_app_service::GalleryScope::IncludeSubfolders,
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
    let asset_id = root_page.items[0].id.clone();
    let child_page = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let page = engine
                .query_wall(
                    &child,
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
    assert_eq!(child_page.items[0].id, asset_id);
    engine
        .request_derivatives(
            &child,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();
    let key = engine
        .query_wall(
            &child,
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
    let cache_path = config.cache_dir().join(&record.relative_cache_path);
    std::fs::write(&cache_path, b"corrupt").unwrap();
    engine
        .request_derivatives(
            &root,
            photo_app_service::GalleryScope::IncludeSubfolders,
            photo_app_service::DerivativeRequest::visible(vec![asset_id]),
        )
        .await
        .unwrap();

    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert_eq!(catalog.derivative_count(child.group_id(), true).unwrap(), 1);
    assert_eq!(catalog.derivative_count(root.group_id(), true).unwrap(), 1);
    assert!(image::load_from_memory(&std::fs::read(cache_path).unwrap()).is_ok());
}

#[tokio::test]
async fn hosted_encode_failure_reaches_the_request_as_derivative_failed() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    let photo = source.join("photo.jpg");
    image::ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(&photo)
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config, source.clone()).unwrap();
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
    std::fs::write(photo, b"not a jpeg").unwrap();
    let error = engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![page.items[0].id.clone()]),
        )
        .await
        .expect_err("decode failures must not be reported as cache unavailability");
    assert!(matches!(
        error,
        photo_app_service::AppServiceError::DerivativeFailed
    ));
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_supervisor_recovers_after_pre_admission_panic_without_aborting_sibling() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    for name in ["first.jpg", "second.jpg"] {
        ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
            .save(source.join(name))
            .unwrap();
    }
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
            if page.items.len() == 2 {
                break page;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let first = page
        .items
        .iter()
        .find(|item| item.display_name == "first.jpg")
        .unwrap()
        .id
        .clone();
    let second = page
        .items
        .iter()
        .find(|item| item.display_name == "second.jpg")
        .unwrap()
        .id
        .clone();
    engine
        .install_hosted_derivative_panic_before_admission_test_hook()
        .await;
    let first_engine = engine.clone();
    let first_selection = selection.clone();
    let first_request = tokio::spawn(async move {
        first_engine
            .request_derivatives(
                &first_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![first]),
            )
            .await
    });
    let second_engine = engine.clone();
    let second_selection = selection.clone();
    let second_request = tokio::spawn(async move {
        second_engine
            .request_derivatives(
                &second_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![second]),
            )
            .await
    });
    let first_result = tokio::time::timeout(std::time::Duration::from_secs(5), first_request)
        .await
        .unwrap()
        .unwrap();
    let second_result = tokio::time::timeout(std::time::Duration::from_secs(5), second_request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        first_result,
        Err(photo_app_service::AppServiceError::DerivativeFailed)
    ));
    assert!(second_result.is_ok(), "queued sibling must get a successor");
    assert!(engine.hosted_derivative_attempts_for_test() >= 1);
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_supervisor_drop_restarts_queued_work_and_settles_active_waiter() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    for name in ["first.jpg", "second.jpg"] {
        ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
            .save(source.join(name))
            .unwrap();
    }
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
            if page.items.len() == 2 {
                break page;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let first = page
        .items
        .iter()
        .find(|item| item.display_name == "first.jpg")
        .unwrap()
        .id
        .clone();
    let second = page
        .items
        .iter()
        .find(|item| item.display_name == "second.jpg")
        .unwrap()
        .id
        .clone();
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_commit_post_publication_test_gate(entered.clone(), release.clone())
        .await;
    let first_engine = engine.clone();
    let first_selection = selection.clone();
    let first_request = tokio::spawn(async move {
        first_engine
            .request_derivatives(
                &first_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![first]),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .expect("active hosted attempt did not reach barrier");
    let second_engine = engine.clone();
    let second_selection = selection.clone();
    let second_request = tokio::spawn(async move {
        second_engine
            .request_derivatives(
                &second_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![second]),
            )
            .await
    });
    tokio::task::yield_now().await;
    engine.abort_hosted_derivative_driver_for_test().await;
    release.notify_waiters();
    let first_result = tokio::time::timeout(std::time::Duration::from_secs(5), first_request)
        .await
        .unwrap()
        .unwrap();
    let second_result = tokio::time::timeout(std::time::Duration::from_secs(5), second_request)
        .await
        .unwrap()
        .unwrap();
    // The active attempt may finish its already-admitted commit, or the
    // owner cleanup may settle it as failed; either outcome must terminate.
    let _ = first_result;
    assert!(second_result.is_ok(), "successor must process queued work");
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_driver_drop_aborts_active_attempt_without_stale_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    for name in ["first.jpg", "second.jpg"] {
        ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
            .save(source.join(name))
            .unwrap();
    }
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
            if page.items.len() == 2 {
                break page;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let first = page
        .items
        .iter()
        .find(|item| item.display_name == "first.jpg")
        .unwrap()
        .id
        .clone();
    let second = page
        .items
        .iter()
        .find(|item| item.display_name == "second.jpg")
        .unwrap()
        .id
        .clone();
    let watermark = engine.current_event_id_for_test(&selection);
    let mut updates = engine.subscribe(
        &selection,
        "drop-no-stale-ready".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(watermark),
    );
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_commit_publication_test_gate(entered.clone(), release.clone())
        .await;
    let first_engine = engine.clone();
    let first_selection = selection.clone();
    let first_for_request = first.clone();
    let first_request = tokio::spawn(async move {
        first_engine
            .request_derivatives(
                &first_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![first_for_request]),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .expect("active attempt did not reach commit barrier");

    let second_engine = engine.clone();
    let second_selection = selection.clone();
    let second_for_request = second.clone();
    let second_request = tokio::spawn(async move {
        second_engine
            .request_derivatives(
                &second_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![second_for_request]),
            )
            .await
    });
    tokio::task::yield_now().await;
    engine.abort_hosted_derivative_driver_for_test().await;
    release.notify_waiters();

    let first_result = tokio::time::timeout(std::time::Duration::from_secs(5), first_request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        first_result,
        Err(photo_app_service::AppServiceError::DerivativeUnavailable)
    ));
    assert!(second_request.await.unwrap().is_ok());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = updates.recv().await.unwrap();
            if let photo_app_service::WallUpdate::DerivativesReady { derivatives, .. } =
                event.update
            {
                assert!(
                    derivatives
                        .iter()
                        .all(|derivative| derivative.asset_id != first)
                );
                if derivatives
                    .iter()
                    .any(|derivative| derivative.asset_id == second)
                {
                    break;
                }
            }
        }
    })
    .await
    .expect("queued successor did not publish its ready event");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if engine.hosted_derivative_active_count_for_test() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled child did not terminate");
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_membership_fence_rejects_staged_bytes_before_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    let photo = source.join("photo.jpg");
    ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(&photo)
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source).unwrap();
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
    let asset_id =
        photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&page.items[0].id).unwrap());
    let event_watermark = engine.current_event_id_for_test(&selection);
    let mut updates = engine.subscribe(
        &selection,
        "publication-fence".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(event_watermark),
    );
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_commit_publication_test_gate(entered.clone(), release.clone())
        .await;
    let request_engine = engine.clone();
    let request_selection = selection.clone();
    let request_asset_id = page.items[0].id.clone();
    let request = tokio::spawn(async move {
        request_engine
            .request_derivatives(
                &request_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![request_asset_id]),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .expect("hosted commit did not reach the publication fence");

    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let existing = catalog.find_asset(asset_id).unwrap().unwrap();
    catalog
        .upsert_asset(&photo_catalog::NewAsset {
            id: existing.id,
            library_id: existing.library_id,
            relative_path: existing.relative_path,
            display_path: existing.display_path,
            media_kind: existing.media_kind,
            signature: photo_domain::FileSignature {
                size_bytes: existing.signature.size_bytes,
                modified_unix_ns: existing.signature.modified_unix_ns.wrapping_add(1),
                sidecar_modified_unix_ns: existing.signature.sidecar_modified_unix_ns,
            },
            folder_group_id: None,
        })
        .unwrap();
    release.notify_waiters();

    assert!(matches!(
        request.await.unwrap().unwrap_err(),
        photo_app_service::AppServiceError::DerivativeUnavailable
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), updates.recv())
            .await
            .is_err(),
        "stale staged work must not publish a ready event"
    );
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert!(catalog.all_derivatives().unwrap().is_empty());
}

#[tokio::test]
async fn hosted_repair_rejects_same_size_jpeg_with_corrupt_entropy() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    let source_image = image::ImageBuffer::from_fn(128, 96, |x, y| {
        image::Rgb([
            (x.wrapping_mul(17) ^ y.wrapping_mul(3)) as u8,
            (x.wrapping_mul(5) ^ y.wrapping_mul(11)) as u8,
            (x.wrapping_mul(13) ^ y.wrapping_mul(7)) as u8,
        ])
    });
    source_image.save(source.join("photo.jpg")).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source).unwrap();
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
    let valid = std::fs::read(&cache_path).unwrap();
    assert!(
        valid
            .windows(2)
            .position(|window| window == [0xff, 0xda])
            .is_some()
    );
    let sos = valid
        .windows(2)
        .position(|window| window == [0xff, 0xda])
        .expect("generated JPEG has scan");
    let scan_length = usize::from(u16::from_be_bytes([valid[sos + 2], valid[sos + 3]]));
    let entropy_start = sos + 2 + scan_length;
    let entropy_end = valid.len() - 2;
    assert!(entropy_start < entropy_end);
    let dht = valid
        .windows(2)
        .position(|window| window == [0xff, 0xc4])
        .expect("generated JPEG has Huffman tables");
    let mut corrupt = valid.clone();
    // Keep every marker and segment length intact while making the frame
    // content undecodable. Marker-only validation accepts this file.
    corrupt[entropy_start] ^= 0x01;
    corrupt[dht + 5] = 0xff;
    assert_eq!(corrupt.len(), valid.len());
    assert!(image::load_from_memory(&corrupt).is_err());
    std::fs::write(&cache_path, corrupt).unwrap();

    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id]),
        )
        .await
        .unwrap();
    assert!(image::load_from_memory(&std::fs::read(cache_path).unwrap()).is_ok());
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_membership_removal_at_publication_leaves_no_catalog_link_or_event() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    image::ImageBuffer::from_pixel(32, 24, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("photo.jpg"))
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source).unwrap();
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
    let asset_id =
        photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&page.items[0].id).unwrap());
    let mut updates = engine.subscribe(
        &selection,
        "membership-removal".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(engine.current_event_id_for_test(&selection)),
    );
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_commit_publication_test_gate(entered.clone(), release.clone())
        .await;
    let request_engine = engine.clone();
    let request_selection = selection.clone();
    let request_asset = page.items[0].id.clone();
    let request = tokio::spawn(async move {
        request_engine
            .request_derivatives(
                &request_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![request_asset]),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .expect("hosted commit did not reach publication barrier");
    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    catalog
        .remove_asset_membership(selection.group_id(), asset_id)
        .unwrap();
    release.notify_waiters();

    assert!(matches!(
        request.await.unwrap().unwrap_err(),
        photo_app_service::AppServiceError::DerivativeUnavailable
    ));
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert!(catalog.all_derivatives().unwrap().is_empty());
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), updates.recv())
            .await
            .is_err()
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_cancellation_keeps_blocking_encode_admitted_until_successor_runs() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    for name in ["first.jpg", "second.jpg"] {
        image::ImageBuffer::from_pixel(512, 384, image::Rgb([220_u8, 180_u8, 80_u8]))
            .save(source.join(name))
            .unwrap();
    }
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
            if page.items.len() == 2 {
                break page;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let first = page
        .items
        .iter()
        .find(|item| item.display_name == "first.jpg")
        .unwrap()
        .id
        .clone();
    let second = page
        .items
        .iter()
        .find(|item| item.display_name == "second.jpg")
        .unwrap()
        .id
        .clone();
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    engine
        .install_hosted_encode_test_gate(entered.clone(), release.clone())
        .await;
    let mut updates = engine.subscribe(
        &selection,
        "blocking-cancel".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(engine.current_event_id_for_test(&selection)),
    );
    let first_engine = engine.clone();
    let first_selection = selection.clone();
    let first_for_request = first.clone();
    let first_request = tokio::spawn(async move {
        first_engine
            .request_derivatives(
                &first_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![first_for_request]),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .expect("encoder did not enter blocking gate");
    let second_engine = engine.clone();
    let second_selection = selection.clone();
    let second_for_request = second.clone();
    let second_request = tokio::spawn(async move {
        second_engine
            .request_derivatives(
                &second_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![second_for_request]),
            )
            .await
    });
    tokio::task::yield_now().await;
    engine.abort_hosted_derivative_driver_for_test().await;
    let first_result = tokio::time::timeout(std::time::Duration::from_secs(5), first_request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        first_result,
        Err(photo_app_service::AppServiceError::DerivativeUnavailable)
    ));
    assert_eq!(engine.hosted_derivative_active_count_for_test(), 1);
    release.store(true, std::sync::atomic::Ordering::Release);
    let second_result = tokio::time::timeout(std::time::Duration::from_secs(5), second_request)
        .await
        .unwrap()
        .unwrap();
    assert!(second_result.is_ok());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if engine.hosted_derivative_active_count_for_test() == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("blocking encoder admission was not released");
    assert!(engine.hosted_derivative_peak_count_for_test() >= 1);
    assert!(
        engine.hosted_derivative_peak_count_for_test()
            <= engine.hosted_derivative_admission_cap_for_test()
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = updates.recv().await.unwrap();
            if let photo_app_service::WallUpdate::DerivativesReady { derivatives, .. } =
                event.update
            {
                assert!(
                    derivatives
                        .iter()
                        .all(|derivative| derivative.asset_id != first)
                );
                if derivatives
                    .iter()
                    .any(|derivative| derivative.asset_id == second)
                {
                    break;
                }
            }
        }
    })
    .await
    .expect("successor did not publish its ready event");
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_repair_failures_keep_the_catalog_link_and_repair_on_retry() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    let source_file = source.join("photo.jpg");
    image::ImageBuffer::from_pixel(32, 24, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(&source_file)
        .unwrap();
    let source_before = std::fs::read(&source_file).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source).unwrap();
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
    let request = photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]);
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            request.clone(),
        )
        .await
        .unwrap();
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let key = catalog.all_derivatives().unwrap()[0].cache_key.clone();
    let relative = catalog.all_derivatives().unwrap()[0]
        .relative_cache_path
        .clone();
    let cache_path = config.cache_dir().join(&relative);

    std::fs::write(&cache_path, b"corrupt").unwrap();
    engine.fail_next_hosted_cleanup_for_test();
    assert!(matches!(
        engine
            .request_derivatives(
                &selection,
                photo_app_service::GalleryScope::CurrentFolder,
                request.clone(),
            )
            .await,
        Err(photo_app_service::AppServiceError::DerivativeFailed)
    ));
    assert_eq!(std::fs::read(&cache_path).unwrap(), b"corrupt");
    assert!(
        photo_catalog::Catalog::open(&config.catalog_path())
            .unwrap()
            .find_derivative_by_cache_key(&key)
            .unwrap()
            .is_some()
    );

    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            request.clone(),
        )
        .await
        .unwrap();
    assert!(image::load_from_memory(&std::fs::read(&cache_path).unwrap()).is_ok());

    std::fs::write(&cache_path, b"corrupt-again").unwrap();
    engine.fail_next_hosted_link_removal_for_test();
    assert!(matches!(
        engine
            .request_derivatives(
                &selection,
                photo_app_service::GalleryScope::CurrentFolder,
                request.clone(),
            )
            .await,
        Err(photo_app_service::AppServiceError::DerivativeFailed)
    ));
    assert_eq!(std::fs::read(&cache_path).unwrap(), b"corrupt-again");
    assert!(
        photo_catalog::Catalog::open(&config.catalog_path())
            .unwrap()
            .find_derivative_by_cache_key(&key)
            .unwrap()
            .is_some()
    );
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            request,
        )
        .await
        .unwrap();
    assert!(image::load_from_memory(&std::fs::read(&cache_path).unwrap()).is_ok());
    assert_eq!(std::fs::read(source_file).unwrap(), source_before);
}
