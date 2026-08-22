use std::path::Path;

use photo_catalog::{Catalog, NewAsset, NewLibrary, SqliteVersion};
use photo_domain::{MediaKind, RelativePathKey};

#[test]
fn opens_with_safe_sqlite_and_round_trips_library_and_asset() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    assert!(catalog.sqlite_version().unwrap() >= SqliteVersion::new(3, 51, 3));

    let library = NewLibrary::configured("Pictures", Path::new("/mounted/Pictures"));
    let stored = catalog.add_library(&library).unwrap();
    let relative = RelativePathKey::from_relative_path(Path::new("2026/Trip/a.jpg")).unwrap();
    let asset = NewAsset::minimal(stored.id, relative, "2026/Trip/a.jpg", MediaKind::Jpeg, 42);
    catalog.upsert_asset(&asset).unwrap();

    assert_eq!(catalog.list_libraries().unwrap(), vec![stored]);
    assert_eq!(
        catalog.find_asset(asset.id).unwrap().unwrap().display_path,
        "2026/Trip/a.jpg"
    );
}

#[test]
fn file_catalog_uses_wal_and_reopens_persisted_records() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("state/catalog.sqlite");
    let expected_id = {
        let mut catalog = Catalog::open(&path).unwrap();
        assert_eq!(catalog.journal_mode().unwrap(), "wal");
        catalog
            .add_library(&NewLibrary::configured(
                "Pictures",
                Path::new("/mounted/Pictures"),
            ))
            .unwrap()
            .id
    };

    let reopened = Catalog::open(&path).unwrap();

    assert_eq!(reopened.list_libraries().unwrap()[0].id, expected_id);
}

#[test]
fn lists_assets_with_stable_keyset_pagination() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured(
            "Pictures",
            Path::new("/mounted/Pictures"),
        ))
        .unwrap();
    for display_path in ["b.jpg", "a.jpg", "c.jpg"] {
        let relative = RelativePathKey::from_relative_path(Path::new(display_path)).unwrap();
        catalog
            .upsert_asset(&NewAsset::minimal(
                library.id,
                relative,
                display_path,
                MediaKind::Jpeg,
                1,
            ))
            .unwrap();
    }

    let first = catalog.list_assets_page(library.id, None, 2).unwrap();
    let cursor = (first[1].display_path.clone(), first[1].id);
    let second = catalog
        .list_assets_page(library.id, Some(cursor), 2)
        .unwrap();

    assert_eq!(
        first
            .iter()
            .map(|asset| asset.display_path.as_str())
            .collect::<Vec<_>>(),
        vec!["a.jpg", "b.jpg"]
    );
    assert_eq!(second[0].display_path, "c.jpg");
}
