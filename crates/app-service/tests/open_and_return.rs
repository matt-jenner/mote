use photo_app_service::{AppConfig, AppService};
use photo_domain::Appearance;

#[test]
fn source_and_appearance_restore_from_the_same_profile() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Iceland 2025");
    std::fs::create_dir(&photos).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));

    let mut first = AppService::open(config.clone()).unwrap();
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
fn bootstrap_does_not_expose_a_native_source_path() {
    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("Private Folder Name");
    std::fs::create_dir(&photos).unwrap();
    let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
    let mut service = AppService::open(config).unwrap();
    let state = service.open_recent(&photos).unwrap();

    let json = serde_json::to_string(&state).unwrap();
    assert!(json.contains("Private Folder Name"));
    assert!(!json.contains(&temp.path().to_string_lossy().to_string()));
}
