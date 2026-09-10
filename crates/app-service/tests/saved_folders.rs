use photo_app_service::{AppConfig, AppService};

#[tokio::test]
async fn saved_entries_reuse_labels_and_removing_active_clears_the_wall() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Photos");
    std::fs::create_dir(&photos).unwrap();
    let service = AppService::open(AppConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
    ))
    .unwrap();
    let first = service.open_recent(&photos).unwrap();
    let id = &first.saved_folders.entries[0].id;
    service.rename_saved_folder(id, Some("Holiday")).unwrap();
    let reopened = service.open_recent(&photos).unwrap();
    assert_eq!(reopened.saved_folders.entries.len(), 1);
    assert_eq!(reopened.active_source.unwrap().display_name, "Holiday");
    let removed = service.remove_saved_folder(id).unwrap();
    assert!(removed.active_source.is_none());
    assert!(removed.saved_folders.has_opened_folder);
    assert!(service.active_selection_id().is_none());
    assert!(
        service
            .query_wall(photo_app_service::WallQueryRequest::oldest_first())
            .await
            .unwrap()
            .items
            .is_empty()
    );
    let added = service.open_recent(&photos).unwrap();
    assert_ne!(added.saved_folders.entries[0].id, *id);
}

#[tokio::test]
async fn unavailable_entry_does_not_replace_current_selection() {
    let temp = tempfile::tempdir().unwrap();
    let first = temp.path().join("First");
    let second = temp.path().join("Second");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    let service = AppService::open(AppConfig::new(
        temp.path().join("data"),
        temp.path().join("cache"),
    ))
    .unwrap();
    let one = service.open_recent(&first).unwrap();
    let id = one.saved_folders.entries[0].id.clone();
    let two = service.open_recent(&second).unwrap();
    std::fs::remove_dir(&first).unwrap();
    assert!(service.activate_saved_folder(&id).await.unwrap().is_none());
    assert_eq!(
        service.bootstrap().unwrap().active_source,
        two.active_source
    );
    let checked = service.check_saved_folders(&[id]).await.unwrap();
    assert!(checked.access.values().any(|a| matches!(
        a.state,
        photo_app_service::FolderAccessState::Missing
            | photo_app_service::FolderAccessState::RootOffline
    )));
}
