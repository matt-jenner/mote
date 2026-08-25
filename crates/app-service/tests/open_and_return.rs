use photo_app_service::{AppConfig, AppService};
use photo_domain::Appearance;

#[test]
fn source_and_appearance_restore_from_the_same_profile() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Iceland 2025");
    std::fs::create_dir(&photos).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));

    let first = AppService::open(config.clone()).unwrap();
    let selected = first.open_recent(&photos).unwrap();
    assert_eq!(selected.active_source.unwrap().display_name, "Iceland 2025");
    first.update_appearance(Appearance::Dark).unwrap();
    drop(first);

    let reopened = AppService::open(config).unwrap();
    let state = reopened.bootstrap().unwrap();
    assert_eq!(state.settings.appearance, Appearance::Dark);
    assert_eq!(state.active_source.unwrap().display_name, "Iceland 2025");
}

#[test]
fn selecting_a_child_folder_restores_its_basename_after_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let library_root = temp.path().join("Private Library Root");
    let child = library_root.join("Iceland 2025");
    std::fs::create_dir_all(&child).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));

    let first = AppService::open(config.clone()).unwrap();
    first.open_recent(&library_root).unwrap();
    let selected = first.open_recent(&child).unwrap();
    assert_eq!(selected.active_source.unwrap().display_name, "Iceland 2025");
    drop(first);

    let reopened = AppService::open(config).unwrap();
    let state = reopened.bootstrap().unwrap();
    assert_eq!(
        state.active_source.as_ref().unwrap().display_name,
        "Iceland 2025"
    );
    let json = serde_json::to_string(&state).unwrap();
    assert!(!json.contains("Private Library Root"));
    assert!(!json.contains(&temp.path().to_string_lossy().to_string()));
}

#[test]
fn default_profile_reconciliation_preserves_named_profile_partials() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let named_cache = cache.join("profiles/clean-demo");
    std::fs::create_dir_all(&named_cache).unwrap();
    let named_partial = named_cache.join("thumbnail.partial-in-progress");
    std::fs::write(&named_partial, b"named profile work").unwrap();

    let config = AppConfig::new(temp.path().join("data"), cache);
    let _default_profile = AppService::open(config).unwrap();

    assert_eq!(
        std::fs::read(&named_partial).unwrap(),
        b"named profile work"
    );

    let named_config = AppConfig::new(temp.path().join("data/profiles/clean-demo"), named_cache);
    let _named_profile = AppService::open(named_config).unwrap();
    assert!(!named_partial.exists());
}

#[test]
fn bootstrap_does_not_expose_a_native_source_path() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Private Folder Name");
    std::fs::create_dir(&photos).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let service = AppService::open(config).unwrap();
    let state = service.open_recent(&photos).unwrap();

    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("Private Folder Name"));
    assert!(!json.contains(&temp.path().to_string_lossy().to_string()));
}

#[test]
fn cloned_services_share_selection_and_appearance() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Shared Folder");
    std::fs::create_dir(&photos).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let first = AppService::open(config).unwrap();
    let second = first.clone();

    first.open_recent(&photos).unwrap();
    second.update_appearance(Appearance::Dark).unwrap();

    let state = first.bootstrap().unwrap();
    assert_eq!(state.settings.appearance, Appearance::Dark);
    assert_eq!(state.active_source.unwrap().display_name, "Shared Folder");
}

#[test]
fn reopening_the_active_folder_refreshes_group_recency() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Viewed Folder");
    std::fs::create_dir(&photos).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let first = AppService::open(config.clone()).unwrap();
    first.open_recent(&photos).unwrap();
    drop(first);

    let mut catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    let selection = catalog.load_app_state().unwrap().active_selection.unwrap();
    let group = catalog
        .folder_group_for_path(selection.library_id, &selection.relative_folder)
        .unwrap()
        .unwrap();
    catalog.touch_folder_group(group, 1).unwrap();
    drop(catalog);

    let reopened = AppService::open(config.clone()).unwrap();
    drop(reopened);
    let catalog = photo_catalog::Catalog::open(&config.catalog_path()).unwrap();
    assert!(catalog.folder_group_last_viewed_at(group).unwrap().unwrap() > 1);
}
