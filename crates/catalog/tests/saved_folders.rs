use photo_catalog::{Catalog, NewFolderGroup, NewLibrary, StoredSourceSelection};
use photo_domain::{FolderGroupId, RelativePathKey};
use std::path::Path;

fn group(catalog: &mut Catalog) -> FolderGroupId {
    let library = catalog
        .add_library(&NewLibrary::recent("Photos", Path::new("/Photos")))
        .unwrap();
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(Path::new("Family")).unwrap(),
            display_path: "Family".into(),
            last_viewed_at: None,
        })
        .unwrap()
}

#[test]
fn duplicate_save_preserves_label_across_database_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&path).unwrap();
    let group = group(&mut catalog);
    let first = catalog.save_folder(group).unwrap();
    catalog
        .rename_saved_folder(first.id, Some("Favourites"))
        .unwrap();
    let again = catalog.save_folder(group).unwrap();
    assert_eq!(again.id, first.id);
    assert_eq!(again.custom_label.as_deref(), Some("Favourites"));
    drop(catalog);
    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(catalog.list_saved_folders().unwrap(), vec![again]);
    assert!(catalog.has_opened_folder().unwrap());
}

#[test]
fn removing_active_shortcut_clears_selection_but_preserves_catalog() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let id = group(&mut catalog);
    let group = catalog.folder_group(id).unwrap().unwrap();
    let entry = catalog.save_and_activate_folder(id).unwrap();
    assert_eq!(
        catalog.load_app_state().unwrap().active_selection,
        Some(StoredSourceSelection {
            library_id: group.library_id,
            relative_folder: group.relative_path.clone(),
        })
    );
    assert!(catalog.remove_saved_folder(entry.id).unwrap());
    assert!(catalog.load_app_state().unwrap().active_selection.is_none());
    assert!(catalog.list_saved_folders().unwrap().is_empty());
    assert_eq!(catalog.folder_group(id).unwrap(), Some(group.clone()));
    assert!(catalog.find_library(group.library_id).unwrap().is_some());
    assert!(!catalog.remove_saved_folder(entry.id).unwrap());
    let readded = catalog.save_folder(id).unwrap();
    assert_ne!(readded.id, entry.id);
    assert!(readded.custom_label.is_none());
}

#[test]
fn inactive_removal_leaves_active_selection_and_labels_can_reset() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let first_group = group(&mut catalog);
    let first = catalog.save_and_activate_folder(first_group).unwrap();
    let old_state = catalog.load_app_state().unwrap();
    let root = catalog.folder_group(first_group).unwrap().unwrap();
    let second_group = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: root.library_id,
            relative_path: RelativePathKey::from_relative_path(Path::new("Other")).unwrap(),
            display_path: "Other".into(),
            last_viewed_at: None,
        })
        .unwrap();
    let second = catalog.save_folder(second_group).unwrap();
    catalog.rename_saved_folder(first.id, Some("Same")).unwrap();
    catalog
        .rename_saved_folder(second.id, Some("Same"))
        .unwrap();
    catalog.rename_saved_folder(first.id, None).unwrap();
    assert!(!catalog.remove_saved_folder(second.id).unwrap());
    assert_eq!(catalog.load_app_state().unwrap(), old_state);
    assert!(
        catalog.list_saved_folders().unwrap()[0]
            .custom_label
            .is_none()
    );
}
