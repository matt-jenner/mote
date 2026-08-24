use std::path::Path;

use photo_catalog::{Catalog, NewLibrary, StoredSourceSelection};
use photo_domain::{Appearance, RelativePathKey};

#[test]
fn appearance_and_active_selection_survive_reopen() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&path).unwrap();
    let library = catalog
        .add_library(&NewLibrary::recent("Family", Path::new("/Volumes/Family")))
        .unwrap();
    let selection = StoredSourceSelection {
        library_id: library.id,
        relative_folder: RelativePathKey::from_relative_path(Path::new("")).unwrap(),
    };

    catalog.set_appearance(Appearance::Dark).unwrap();
    catalog.set_active_selection(Some(&selection)).unwrap();
    drop(catalog);

    let catalog = Catalog::open(&path).unwrap();
    let state = catalog.load_app_state().unwrap();
    assert_eq!(state.appearance, Appearance::Dark);
    assert_eq!(state.active_selection, Some(selection));
}

#[test]
fn a_new_catalog_defaults_to_system_without_a_selection() {
    let catalog = Catalog::open_in_memory().unwrap();
    assert_eq!(
        catalog.load_app_state().unwrap().appearance,
        Appearance::System
    );
    assert_eq!(catalog.load_app_state().unwrap().active_selection, None);
}
