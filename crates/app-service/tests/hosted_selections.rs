use std::path::Path;

use image::ImageBuffer;
use photo_app_service::{AppConfig, GalleryEngine, GallerySelection};

async fn run_and_wait_for_settlement(
    engine: &GalleryEngine,
    selection: &GallerySelection,
    client: &str,
) {
    let mut events = engine.subscribe(
        selection,
        client.to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        None,
    );
    engine.ensure_running(selection).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await.unwrap().update,
                photo_app_service::WallUpdate::MetadataSettled { .. }
            ) {
                break;
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("selection did not settle for {client}"));
}

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
    let engine = GalleryEngine::open(config.clone(), source).unwrap();
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
    let reopened = GalleryEngine::open(config.clone(), source.clone()).unwrap();
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

    drop(offline_events);
    drop(reopened);
    std::fs::rename(&offline, &source).unwrap();
    photo_catalog::Catalog::open(&config.catalog_path())
        .unwrap()
        .set_library_availability(
            selection.library_id(),
            photo_domain::Availability::Available,
        )
        .unwrap();
    let remounted = GalleryEngine::open(config, source.clone()).unwrap();
    let remounted_summary = remounted.select_relative(Path::new(".")).await.unwrap();
    assert_eq!(remounted_summary.id, summary.id);
    let remounted_selection = remounted.resolve_selection(&summary.id).unwrap();
    let mut recovery_events = remounted.subscribe(
        &remounted_selection,
        "remounted-recovery".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        None,
    );
    remounted
        .ensure_running(&remounted_selection)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = recovery_events.recv().await.unwrap();
            if matches!(
                event.update,
                photo_app_service::WallUpdate::MetadataSettled { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("remounted stable selection did not reconcile offline assets");
    let remounted_page = remounted
        .query_wall(
            &remounted_selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    assert_eq!(remounted_page.items.len(), 1);
    assert_eq!(
        remounted_page.items[0].availability,
        photo_app_service::SourceAvailability::Available
    );
    remounted
        .request_derivatives(
            &remounted_selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![remounted_page.items[0].id.clone()]),
        )
        .await
        .expect("remounted source should generate a previously uncached derivative");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_recovery_requests_admit_one_scan_for_a_stable_selection() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("photo.jpg"))
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    let mut initial_events = engine.subscribe(
        &selection,
        "initial-scan".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        None,
    );
    engine.ensure_running(&selection).await.unwrap();
    loop {
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), initial_events.recv())
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
    let after_initial = engine.current_event_id_for_test(&selection);
    let mut recovery_events = engine.subscribe(
        &selection,
        "recovery-scan".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(after_initial),
    );

    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    catalog.mark_root_offline(selection.library_id()).unwrap();
    catalog
        .set_library_availability(
            selection.library_id(),
            photo_domain::Availability::Available,
        )
        .unwrap();
    drop(catalog);

    let mut requests = Vec::new();
    for _ in 0..8 {
        let engine = engine.clone();
        let selection = selection.clone();
        requests.push(tokio::spawn(async move {
            engine.ensure_running(&selection).await
        }));
    }
    for request in requests {
        request.await.unwrap().unwrap();
    }
    let recovered = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = recovery_events.recv().await.unwrap();
            if matches!(
                event.update,
                photo_app_service::WallUpdate::MetadataSettled { .. }
            ) {
                break;
            }
        }
    })
    .await;
    assert!(
        recovered.is_ok(),
        "stable selection did not run a recovery scan"
    );
    let page = engine
        .query_wall(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    assert_eq!(
        page.items[0].availability,
        photo_app_service::SourceAvailability::Available
    );

    for _ in 0..8 {
        engine.ensure_running(&selection).await.unwrap();
    }
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            recovery_events.recv()
        )
        .await
        .is_err(),
        "one recovery transition admitted more than one scan"
    );
}

#[tokio::test]
async fn recovery_token_rescans_a_completed_empty_selection_once() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    let summary = engine.select_relative(Path::new(".")).await.unwrap();
    let selection = engine.resolve_selection(&summary.id).unwrap();
    let mut events = engine.subscribe(
        &selection,
        "empty-initial".to_owned(),
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
    let after_initial = engine.current_event_id_for_test(&selection);
    let mut recovered_events = engine.subscribe(
        &selection,
        "empty-recovery".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(after_initial),
    );
    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    catalog.mark_root_offline(selection.library_id()).unwrap();
    catalog
        .set_library_availability(
            selection.library_id(),
            photo_domain::Availability::Available,
        )
        .unwrap();
    drop(catalog);
    ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("arrived-while-offline.jpg"))
        .unwrap();
    engine.ensure_running(&selection).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = recovered_events.recv().await.unwrap();
            if matches!(
                event.update,
                photo_app_service::WallUpdate::MetadataSettled { .. }
            ) {
                break;
            }
        }
    })
    .await
    .expect("recovery token did not reconcile a previously empty selection");
    let page = engine
        .query_wall(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(
        page.items[0].availability,
        photo_app_service::SourceAvailability::Available
    );
    engine.ensure_running(&selection).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            recovered_events.recv()
        )
        .await
        .is_err(),
        "a reconciled recovery token admitted another scan"
    );
}

#[tokio::test]
async fn overlapping_group_recovery_survives_restart_and_shared_asset_updates() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let child_source = source.join("child");
    std::fs::create_dir_all(&child_source).unwrap();
    let original = child_source.join("original.jpg");
    ImageBuffer::from_pixel(3, 2, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(&original)
        .unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));

    let engine = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    let parent_summary = engine.select_relative(Path::new("")).await.unwrap();
    let child_summary = engine.select_relative(Path::new("child")).await.unwrap();
    let parent = engine.resolve_selection(&parent_summary.id).unwrap();
    let child = engine.resolve_selection(&child_summary.id).unwrap();
    run_and_wait_for_settlement(&engine, &parent, "parent-initial").await;
    run_and_wait_for_settlement(&engine, &child, "child-initial").await;
    drop(engine);

    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    catalog.mark_root_offline(parent.library_id()).unwrap();
    catalog
        .set_library_availability(parent.library_id(), photo_domain::Availability::Available)
        .unwrap();
    drop(catalog);

    let parent_recovery = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    let parent = parent_recovery
        .resolve_selection(&parent_summary.id)
        .unwrap();
    run_and_wait_for_settlement(&parent_recovery, &parent, "parent-recovery").await;
    let child_state = photo_catalog::Catalog::open(&config.catalog_path())
        .unwrap()
        .folder_group_recovery_state(parent.library_id(), child.group_id())
        .unwrap();
    assert!(
        child_state.requested > child_state.reconciled,
        "recovering the overlapping parent must not erase the child's request"
    );
    let parent_state = photo_catalog::Catalog::open(&config.catalog_path())
        .unwrap()
        .folder_group_recovery_state(parent.library_id(), parent.group_id())
        .unwrap();
    assert_eq!(parent_state.requested, parent_state.reconciled);
    drop(parent_recovery);

    let reconciled_parent = GalleryEngine::open(config.clone(), source.clone()).unwrap();
    let parent = reconciled_parent
        .resolve_selection(&parent_summary.id)
        .unwrap();
    let mut reconciled_events = reconciled_parent.subscribe(
        &parent,
        "parent-reconciled-restart".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        None,
    );
    reconciled_parent.ensure_running(&parent).await.unwrap();
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            reconciled_events.recv()
        )
        .await
        .is_err(),
        "a persisted reconciled token must suppress a duplicate scan after restart"
    );
    drop(reconciled_events);
    drop(reconciled_parent);

    std::fs::remove_file(&original).unwrap();
    ImageBuffer::from_pixel(3, 2, image::Rgb([30_u8, 60_u8, 90_u8]))
        .save(child_source.join("arrived.jpg"))
        .unwrap();

    let child_recovery = GalleryEngine::open(config.clone(), source).unwrap();
    let child_summary_after_restart = child_recovery
        .select_relative(Path::new("child"))
        .await
        .unwrap();
    assert_eq!(child_summary_after_restart.id, child_summary.id);
    let child = child_recovery.resolve_selection(&child_summary.id).unwrap();
    run_and_wait_for_settlement(&child_recovery, &child, "child-recovery").await;
    let page = child_recovery
        .query_wall(
            &child,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::WallQueryRequest::oldest_first(),
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].display_name, "arrived.jpg");
    let reconciled = photo_catalog::Catalog::open(&config.catalog_path())
        .unwrap()
        .folder_group_recovery_state(child.library_id(), child.group_id())
        .unwrap();
    assert_eq!(reconciled.requested, child_state.requested);
    assert_eq!(reconciled.reconciled, reconciled.requested);
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
    let expected_size = record.size_bytes;
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

    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image::ImageBuffer::from_pixel(
        3,
        2,
        image::Rgb([10_u8, 20_u8, 30_u8]),
    ))
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .unwrap();
    assert!(png.len() <= expected_size as usize);
    png.resize(expected_size as usize, 0);
    assert_eq!(image::guess_format(&png).unwrap(), image::ImageFormat::Png);
    assert!(image::load_from_memory(&png).is_ok());
    std::fs::write(&cache_path, png).unwrap();
    engine
        .request_derivatives(
            &selection,
            photo_app_service::GalleryScope::CurrentFolder,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();
    assert_eq!(
        image::guess_format(&std::fs::read(&cache_path).unwrap()).unwrap(),
        image::ImageFormat::Jpeg
    );
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
    let source_before = std::fs::read(source.join("child/photo.jpg")).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let engine = GalleryEngine::open(config.clone(), source.clone()).unwrap();
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
    engine
        .request_derivatives(
            &root,
            photo_app_service::GalleryScope::IncludeSubfolders,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert_eq!(catalog.derivative_count(child.group_id(), true).unwrap(), 1);
    assert_eq!(catalog.derivative_count(root.group_id(), true).unwrap(), 1);

    std::fs::write(&cache_path, b"old-shared-bytes").unwrap();
    engine.fail_next_hosted_link_removal_for_test();
    assert!(matches!(
        engine
            .request_derivatives(
                &root,
                photo_app_service::GalleryScope::IncludeSubfolders,
                photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
            )
            .await,
        Err(photo_app_service::AppServiceError::DerivativeFailed)
    ));
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert_eq!(catalog.derivative_count(child.group_id(), true).unwrap(), 1);
    assert_eq!(catalog.derivative_count(root.group_id(), true).unwrap(), 1);
    assert_eq!(std::fs::read(&cache_path).unwrap(), b"old-shared-bytes");

    engine
        .request_derivatives(
            &root,
            photo_app_service::GalleryScope::IncludeSubfolders,
            photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
        )
        .await
        .unwrap();

    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert_eq!(catalog.derivative_count(child.group_id(), true).unwrap(), 1);
    assert_eq!(catalog.derivative_count(root.group_id(), true).unwrap(), 1);
    assert!(image::load_from_memory(&std::fs::read(&cache_path).unwrap()).is_ok());

    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let record = catalog.find_derivative_by_cache_key(&key).unwrap().unwrap();
    assert_eq!(
        catalog
            .remove_derivative_group_links(&[record.id], &[child.group_id()])
            .unwrap(),
        1
    );
    std::fs::write(&cache_path, b"old-final-link-bytes").unwrap();
    engine.fail_next_hosted_cleanup_for_test();
    assert!(matches!(
        engine
            .request_derivatives(
                &root,
                photo_app_service::GalleryScope::IncludeSubfolders,
                photo_app_service::DerivativeRequest::visible(vec![asset_id.clone()]),
            )
            .await,
        Err(photo_app_service::AppServiceError::DerivativeFailed)
    ));
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert_eq!(catalog.derivative_count(child.group_id(), true).unwrap(), 0);
    assert_eq!(catalog.derivative_count(root.group_id(), true).unwrap(), 1);
    assert_eq!(std::fs::read(&cache_path).unwrap(), b"old-final-link-bytes");

    engine
        .request_derivatives(
            &root,
            photo_app_service::GalleryScope::IncludeSubfolders,
            photo_app_service::DerivativeRequest::visible(vec![asset_id]),
        )
        .await
        .unwrap();
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert_eq!(catalog.derivative_count(child.group_id(), true).unwrap(), 0);
    assert_eq!(catalog.derivative_count(root.group_id(), true).unwrap(), 1);
    assert!(image::load_from_memory(&std::fs::read(&cache_path).unwrap()).is_ok());
    assert_eq!(
        std::fs::read(source.join("child/photo.jpg")).unwrap(),
        source_before
    );
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
async fn assert_post_authorization_fault_is_typed_and_recovered(panic: bool) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    for name in ["first.jpg", "second.jpg"] {
        ImageBuffer::from_pixel(64, 48, image::Rgb([220_u8, 180_u8, 80_u8]))
            .save(source.join(name))
            .unwrap();
    }
    let sentinel = source.join("source-sentinel.txt");
    std::fs::write(&sentinel, b"source-owned").unwrap();
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
    let first_id = photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&first).unwrap());
    let second_id = photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&second).unwrap());
    let mut updates = engine.subscribe(
        &selection,
        "post-authorization-fault".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(engine.current_event_id_for_test(&selection)),
    );
    let authorized_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let authorized_release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_authorized_publication_test_gate(
            authorized_entered.clone(),
            authorized_release.clone(),
        )
        .await;
    if panic {
        engine.install_hosted_derivative_panic_after_authorization_test_hook();
    } else {
        engine.install_hosted_derivative_drop_after_authorization_test_hook();
    }

    let first_engine = engine.clone();
    let first_selection = selection.clone();
    let first_request_asset = first.clone();
    let first_request = tokio::spawn(async move {
        first_engine
            .request_derivatives(
                &first_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![first_request_asset]),
            )
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        authorized_entered.notified(),
    )
    .await
    .expect("attempt did not authorize publication before the injected fault");

    let second_engine = engine.clone();
    let second_selection = selection.clone();
    let second_request_asset = second.clone();
    let second_request = tokio::spawn(async move {
        second_engine
            .request_derivatives(
                &second_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![second_request_asset]),
            )
            .await
    });
    authorized_release.notify_waiters();

    let first_result = tokio::time::timeout(std::time::Duration::from_secs(5), first_request)
        .await
        .expect("faulted request retained its captured sender")
        .unwrap();
    let second_result = tokio::time::timeout(std::time::Duration::from_secs(5), second_request)
        .await
        .expect("queued sibling did not complete")
        .unwrap();
    assert!(second_result.is_ok(), "queued sibling must still complete");
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let photo_app_service::WallUpdate::DerivativesReady { derivatives, .. } =
                updates.recv().await.unwrap().update
            {
                assert!(
                    derivatives
                        .iter()
                        .all(|derivative| derivative.asset_id != first),
                    "faulted publication emitted a stale ready event"
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
    .expect("queued sibling did not emit its ready event");
    let pending_jobs = engine.hosted_pending_job_count_for_test(&selection).await;
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert!(
        catalog
            .derivatives_for_assets(&[first_id], "wall_thumbnail")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        catalog
            .derivatives_for_assets(&[second_id], "wall_thumbnail")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(std::fs::read(sentinel).unwrap(), b"source-owned");
    assert!(
        matches!(
            first_result,
            Err(photo_app_service::AppServiceError::DerivativeFailed)
        ) && pending_jobs == 0,
        "post-authorization fault returned {first_result:?} and retained {pending_jobs} job(s)"
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_post_authorization_panic_delivers_typed_failure_and_clears_job() {
    assert_post_authorization_fault_is_typed_and_recovered(true).await;
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn hosted_post_authorization_drop_delivers_typed_failure_and_clears_job() {
    assert_post_authorization_fault_is_typed_and_recovered(false).await;
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
    assert!(
        first_result.is_ok(),
        "an authorized publication must retain ownership through terminal settlement"
    );
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

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hosted_cancellation_transition_yields_to_publication_authorized_after_drop_starts() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    image::ImageBuffer::from_pixel(64, 48, image::Rgb([220_u8, 180_u8, 80_u8]))
        .save(source.join("photo.jpg"))
        .unwrap();
    let sentinel = source.join("source-sentinel.txt");
    std::fs::write(&sentinel, b"source-owned").unwrap();
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
    let asset = page.items[0].id.clone();
    let asset_id = photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&asset).unwrap());
    let mut updates = engine.subscribe(
        &selection,
        "publication-transition".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(engine.current_event_id_for_test(&selection)),
    );

    let publication_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let publication_release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_commit_publication_test_gate(
            publication_entered.clone(),
            publication_release.clone(),
        )
        .await;
    let authorized_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let authorized_release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_authorized_publication_test_gate(
            authorized_entered.clone(),
            authorized_release.clone(),
        )
        .await;

    let request_engine = engine.clone();
    let request_selection = selection.clone();
    let request_asset = asset.clone();
    let request = tokio::spawn(async move {
        request_engine
            .request_derivatives(
                &request_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![request_asset]),
            )
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        publication_entered.notified(),
    )
    .await
    .expect("attempt did not reach the pre-publication barrier");

    let cancellation_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let cancellation_release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancellation_applied = std::sync::Arc::new(tokio::sync::Notify::new());
    let cancellation_completed = std::sync::Arc::new(tokio::sync::Notify::new());
    engine.install_hosted_cancellation_transition_test_gate(
        cancellation_entered.clone(),
        cancellation_release.clone(),
        cancellation_applied.clone(),
        cancellation_completed,
    );
    engine.abort_hosted_derivative_driver_for_test().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cancellation_entered.notified(),
    )
    .await
    .expect("driver drop did not enter its cancellation transition");

    publication_release.notify_waiters();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        authorized_entered.notified(),
    )
    .await
    .expect("attempt did not authorize publication during cancellation transition");
    cancellation_release.store(true, std::sync::atomic::Ordering::Release);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cancellation_applied.notified(),
    )
    .await
    .expect("driver drop did not apply its cancellation decision");
    authorized_release.notify_waiters();

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), request)
        .await
        .expect("authorized request remained retained")
        .unwrap();
    assert!(
        result.is_ok(),
        "publication authorization must win the cancellation transition"
    );
    let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let photo_app_service::WallUpdate::DerivativesReady { derivatives, .. } =
                updates.recv().await.unwrap().update
                && derivatives
                    .iter()
                    .any(|derivative| derivative.asset_id == asset)
            {
                break;
            }
        }
    })
    .await;
    assert!(event.is_ok(), "authorized publication did not emit ready");
    assert_eq!(
        engine.hosted_pending_job_count_for_test(&selection).await,
        0
    );
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert_eq!(
        catalog
            .derivatives_for_assets(&[asset_id], "wall_thumbnail")
            .unwrap()
            .len(),
        1
    );
    assert_eq!(std::fs::read(sentinel).unwrap(), b"source-owned");
}

#[cfg(debug_assertions)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hosted_cancellation_rejects_encode_registration_after_successor_handoff() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    for name in ["first.jpg", "second.jpg"] {
        image::ImageBuffer::from_pixel(64, 48, image::Rgb([220_u8, 180_u8, 80_u8]))
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
    engine.set_hosted_derivative_admission_cap_for_test(4);

    let registration_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let registration_release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let registered = std::sync::Arc::new(tokio::sync::Notify::new());
    let rejected = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_encode_registration_test_gate(
            registration_entered.clone(),
            registration_release.clone(),
            registered.clone(),
            rejected.clone(),
        )
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
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        registration_entered.notified(),
    )
    .await
    .expect("attempt did not reach the encode-registration barrier");

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
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if engine.hosted_waiter_count_for_test(&selection).await == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("successor waiter was not queued");

    let cancellation_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let cancellation_release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancellation_applied = std::sync::Arc::new(tokio::sync::Notify::new());
    let cancellation_completed = std::sync::Arc::new(tokio::sync::Notify::new());
    engine.install_hosted_cancellation_transition_test_gate(
        cancellation_entered.clone(),
        cancellation_release.clone(),
        cancellation_applied.clone(),
        cancellation_completed.clone(),
    );
    engine.abort_hosted_derivative_driver_for_test().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cancellation_entered.notified(),
    )
    .await
    .expect("driver drop did not enter its cancellation transition");
    cancellation_release.store(true, std::sync::atomic::Ordering::Release);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cancellation_applied.notified(),
    )
    .await
    .expect("driver drop did not abort-mark the attempt");
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        cancellation_completed.notified(),
    )
    .await
    .expect("driver drop did not hand ownership to the queued successor");

    let registered_wait = registered.notified();
    let rejected_wait = rejected.notified();
    tokio::pin!(registered_wait);
    tokio::pin!(rejected_wait);
    registered_wait.as_mut().enable();
    rejected_wait.as_mut().enable();
    registration_release.store(true, std::sync::atomic::Ordering::Release);
    let registration_rejected = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::select! {
            _ = &mut registered_wait => false,
            _ = &mut rejected_wait => true,
        }
    })
    .await
    .expect("encode registration produced no terminal transition");

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
        Err(photo_app_service::AppServiceError::DerivativeUnavailable)
    ));
    assert!(second_result.is_ok());
    assert!(
        registration_rejected,
        "an abort-marked attempt registered blocking work after successor handoff"
    );
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
    engine.set_hosted_derivative_admission_cap_for_test(4);
    assert!(
        engine.hosted_derivative_admission_cap_for_test() > 1,
        "the regression needs spare global capacity"
    );
    let entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    engine
        .install_hosted_encode_test_gate(entered.clone(), release.clone())
        .await;
    let successor_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let successor_release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    engine
        .install_hosted_encode_test_gate(successor_entered.clone(), successor_release.clone())
        .await;
    let mut updates = engine.subscribe(
        &selection,
        "blocking-cancel".to_owned(),
        photo_app_service::GalleryScope::CurrentFolder,
        Some(engine.current_event_id_for_test(&selection)),
    );
    engine
        .set_interaction_for_test(photo_app_service::InteractionState::Idle)
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
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if engine.hosted_waiter_count_for_test(&selection).await == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("queued successor waiter was not admitted before cancellation");
    engine.abort_hosted_derivative_driver_for_test().await;
    let successor_started_early = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        successor_entered.notified(),
    )
    .await
    .is_ok();
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert!(catalog.all_derivatives().unwrap().is_empty());
    release.store(true, std::sync::atomic::Ordering::Release);
    if !successor_started_early {
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            successor_entered.notified(),
        )
        .await
        .expect(
            "successor encoder did not start after cancellation cleanup joined the old encoder",
        );
    }
    successor_release.store(true, std::sync::atomic::Ordering::Release);
    let first_result = tokio::time::timeout(std::time::Duration::from_secs(5), first_request)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        first_result,
        Err(photo_app_service::AppServiceError::DerivativeUnavailable)
    ));
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
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let first_id = photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&first).unwrap());
    let second_id = photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&second).unwrap());
    assert!(
        catalog
            .derivatives_for_assets(&[first_id], "wall_thumbnail")
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        catalog
            .derivatives_for_assets(&[second_id], "wall_thumbnail")
            .unwrap()
            .len(),
        1
    );
    assert!(
        !successor_started_early,
        "a successor encoder started before the cancelled encoder exited"
    );
}

#[cfg(debug_assertions)]
#[tokio::test]
async fn authorized_late_waiter_at_empty_commit_boundary_allows_publication() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir_all(&source).unwrap();
    image::ImageBuffer::from_pixel(64, 48, image::Rgb([220_u8, 180_u8, 80_u8]))
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
    let asset = page.items[0].id.clone();
    let asset_id = photo_domain::AssetId::from_uuid(uuid::Uuid::parse_str(&asset).unwrap());
    let publication_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let publication_release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_commit_publication_test_gate(
            publication_entered.clone(),
            publication_release.clone(),
        )
        .await;
    let empty_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let empty_release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_empty_authorization_test_gate(empty_entered.clone(), empty_release.clone())
        .await;
    let encode_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let encode_release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    engine
        .install_hosted_encode_test_gate(encode_entered.clone(), encode_release.clone())
        .await;
    let first_engine = engine.clone();
    let first_selection = selection.clone();
    let first_asset = asset.clone();
    let first = tokio::spawn(async move {
        first_engine
            .request_derivatives(
                &first_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![first_asset]),
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), encode_entered.notified())
        .await
        .expect("initial encode did not reach its gate");
    let pre_enqueue_entered = std::sync::Arc::new(tokio::sync::Notify::new());
    let pre_enqueue_release = std::sync::Arc::new(tokio::sync::Notify::new());
    engine
        .install_hosted_pre_enqueue_test_gate(
            pre_enqueue_entered.clone(),
            pre_enqueue_release.clone(),
        )
        .await;
    let late_engine = engine.clone();
    let late_selection = selection.clone();
    let late_asset = asset.clone();
    let late = tokio::spawn(async move {
        late_engine
            .request_derivatives(
                &late_selection,
                photo_app_service::GalleryScope::CurrentFolder,
                photo_app_service::DerivativeRequest::visible(vec![late_asset]),
            )
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        pre_enqueue_entered.notified(),
    )
    .await
    .expect("late request did not reach the pre-enqueue boundary");
    encode_release.store(true, std::sync::atomic::Ordering::Release);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        publication_entered.notified(),
    )
    .await
    .expect("commit did not reach the membership mutation boundary");
    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert!(
        catalog
            .remove_asset_membership(selection.group_id(), asset_id)
            .unwrap()
    );
    publication_release.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(5), empty_entered.notified())
        .await
        .expect("commit did not observe an empty authorized waiter set");

    catalog
        .add_asset_membership(selection.group_id(), asset_id, 1)
        .unwrap();
    pre_enqueue_release.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if engine.hosted_waiter_count_for_test(&selection).await == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("late authorized waiter was not accepted during early committing");
    empty_release.notify_waiters();

    assert!(first.await.unwrap().is_ok());
    assert!(late.await.unwrap().is_ok());
    assert_eq!(
        photo_catalog::Catalog::open(&config.catalog_path())
            .unwrap()
            .all_derivatives()
            .unwrap()
            .len(),
        1
    );
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
