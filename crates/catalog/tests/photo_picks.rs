use std::path::Path;

use photo_catalog::{Catalog, NewAsset, NewFolderGroup, NewLibrary};
use photo_domain::{AssetId, FolderGroupId, MediaKind, NativePathKey, RelativePathKey};
use rusqlite::Connection;

struct Fixture {
    catalog: Catalog,
    library: photo_domain::LibraryId,
    group: FolderGroupId,
    saved_id: uuid::Uuid,
}

impl Fixture {
    fn new() -> Self {
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = catalog
            .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
            .unwrap();
        let group = catalog
            .upsert_folder_group(&NewFolderGroup {
                id: FolderGroupId::new(),
                library_id: library.id,
                relative_path: RelativePathKey::from_relative_path(Path::new("Family")).unwrap(),
                display_path: "Family".into(),
                last_viewed_at: None,
            })
            .unwrap();
        let saved_id = catalog.save_folder(group).unwrap().id;

        Self {
            catalog,
            library: library.id,
            group,
            saved_id,
        }
    }

    fn asset(&mut self, path: &str) -> AssetId {
        let relative_path = RelativePathKey::from_relative_path(Path::new(path)).unwrap();
        let asset = NewAsset {
            folder_group_id: Some(self.group),
            ..NewAsset::minimal(self.library, relative_path, path, MediaKind::Jpeg, 1)
        };
        self.catalog.upsert_asset(&asset).unwrap();
        asset.id
    }
}

#[test]
fn picks_are_ordered_unique_and_independent_of_saved_shortcuts() {
    let mut fixture = Fixture::new();
    let first = fixture.asset("one.jpg");
    let second = fixture.asset("two.jpg");

    assert!(
        fixture
            .catalog
            .add_photo_pick(first, fixture.group)
            .unwrap()
    );
    assert!(
        fixture
            .catalog
            .add_photo_pick(second, fixture.group)
            .unwrap()
    );
    assert!(
        !fixture
            .catalog
            .add_photo_pick(first, fixture.group)
            .unwrap()
    );
    fixture
        .catalog
        .remove_saved_folder(fixture.saved_id)
        .unwrap();

    assert_eq!(
        fixture
            .catalog
            .list_photo_picks()
            .unwrap()
            .into_iter()
            .map(|pick| pick.asset_id)
            .collect::<Vec<_>>(),
        vec![first, second],
    );
}

#[test]
fn removing_and_clearing_picks_returns_the_removed_entries() {
    let mut fixture = Fixture::new();
    let first = fixture.asset("one.jpg");
    let second = fixture.asset("two.jpg");
    fixture
        .catalog
        .add_photo_pick(first, fixture.group)
        .unwrap();
    fixture
        .catalog
        .add_photo_pick(second, fixture.group)
        .unwrap();

    assert!(fixture.catalog.remove_photo_pick(first).unwrap());
    assert!(!fixture.catalog.remove_photo_pick(first).unwrap());
    assert_eq!(
        fixture
            .catalog
            .list_photo_picks()
            .unwrap()
            .into_iter()
            .map(|pick| pick.asset_id)
            .collect::<Vec<_>>(),
        vec![second],
    );

    let cleared = fixture.catalog.clear_photo_picks().unwrap();
    assert_eq!(
        cleared
            .into_iter()
            .map(|pick| pick.asset_id)
            .collect::<Vec<_>>(),
        vec![second],
    );
    assert!(fixture.catalog.list_photo_picks().unwrap().is_empty());
}

#[test]
fn restoring_cleared_picks_keeps_new_picks_after_their_old_order() {
    let mut fixture = Fixture::new();
    let first = fixture.asset("one.jpg");
    let second = fixture.asset("two.jpg");
    let later = fixture.asset("three.jpg");
    fixture
        .catalog
        .add_photo_pick(first, fixture.group)
        .unwrap();
    fixture
        .catalog
        .add_photo_pick(second, fixture.group)
        .unwrap();
    let cleared = fixture.catalog.clear_photo_picks().unwrap();
    fixture
        .catalog
        .add_photo_pick(later, fixture.group)
        .unwrap();

    fixture.catalog.restore_photo_picks(&cleared).unwrap();

    assert_eq!(
        fixture
            .catalog
            .list_photo_picks()
            .unwrap()
            .into_iter()
            .map(|pick| pick.asset_id)
            .collect::<Vec<_>>(),
        vec![first, second, later],
    );
}

#[test]
fn pick_revision_changes_only_when_pick_membership_changes() {
    let mut fixture = Fixture::new();
    let first = fixture.asset("one.jpg");
    assert_eq!(fixture.catalog.photo_pick_revision().unwrap(), 0);

    assert!(
        fixture
            .catalog
            .add_photo_pick(first, fixture.group)
            .unwrap()
    );
    assert_eq!(fixture.catalog.photo_pick_revision().unwrap(), 1);
    assert!(
        !fixture
            .catalog
            .add_photo_pick(first, fixture.group)
            .unwrap()
    );
    assert_eq!(fixture.catalog.photo_pick_revision().unwrap(), 1);
    assert!(fixture.catalog.remove_photo_pick(first).unwrap());
    assert_eq!(fixture.catalog.photo_pick_revision().unwrap(), 2);
    assert!(fixture.catalog.clear_photo_picks().unwrap().is_empty());
    assert_eq!(fixture.catalog.photo_pick_revision().unwrap(), 2);
}

#[test]
fn copy_destination_round_trips_as_a_native_path_key() {
    let mut fixture = Fixture::new();
    let destination = NativePathKey::from_path(Path::new("/Users/example/Exports"));

    assert_eq!(fixture.catalog.last_copy_destination().unwrap(), None);
    fixture
        .catalog
        .set_last_copy_destination(&destination)
        .unwrap();
    assert_eq!(
        fixture.catalog.last_copy_destination().unwrap(),
        Some(destination),
    );
}

#[test]
fn picks_and_copy_destination_survive_file_catalog_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("catalog.sqlite");
    let destination = NativePathKey::from_path(Path::new("/Users/example/Exports"));

    let (first, second, group) = {
        let mut catalog = Catalog::open(&path).unwrap();
        let library = catalog
            .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
            .unwrap();
        let group = catalog
            .upsert_folder_group(&NewFolderGroup {
                id: FolderGroupId::new(),
                library_id: library.id,
                relative_path: RelativePathKey::from_relative_path(Path::new("Family")).unwrap(),
                display_path: "Family".into(),
                last_viewed_at: None,
            })
            .unwrap();
        let first = NewAsset {
            folder_group_id: Some(group),
            ..NewAsset::minimal(
                library.id,
                RelativePathKey::from_relative_path(Path::new("one.jpg")).unwrap(),
                "one.jpg",
                MediaKind::Jpeg,
                1,
            )
        };
        let second = NewAsset {
            folder_group_id: Some(group),
            ..NewAsset::minimal(
                library.id,
                RelativePathKey::from_relative_path(Path::new("two.jpg")).unwrap(),
                "two.jpg",
                MediaKind::Jpeg,
                1,
            )
        };
        catalog.upsert_asset(&first).unwrap();
        catalog.upsert_asset(&second).unwrap();
        assert!(catalog.add_photo_pick(first.id, group).unwrap());
        assert!(catalog.add_photo_pick(second.id, group).unwrap());
        catalog.set_last_copy_destination(&destination).unwrap();
        (first.id, second.id, group)
    };

    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(catalog.photo_pick_revision().unwrap(), 2);
    assert_eq!(
        catalog
            .list_photo_picks()
            .unwrap()
            .into_iter()
            .map(|pick| (pick.asset_id, pick.folder_group_id))
            .collect::<Vec<_>>(),
        vec![(first, group), (second, group)],
    );
    assert_eq!(catalog.last_copy_destination().unwrap(), Some(destination));
}

#[test]
fn schema_twelve_catalog_upgrades_once_and_reopens_cleanly() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("schema-twelve.sqlite");
    let connection = Connection::open(&path).unwrap();
    for migration in [
        include_str!("../migrations/0001_catalog.sql"),
        include_str!("../migrations/0002_unavailable_assets.sql"),
        include_str!("../migrations/0003_app_state.sql"),
        include_str!("../migrations/0004_wall_projection.sql"),
        include_str!("../migrations/0005_group_scoped_generations.sql"),
        include_str!("../migrations/0006_wall_state_indexes.sql"),
        include_str!("../migrations/0007_derivative_coordinator.sql"),
        include_str!("../migrations/0008_gallery_scope.sql"),
        include_str!("../migrations/0009_selection_membership.sql"),
        include_str!("../migrations/0010_folder_recovery.sql"),
        include_str!("../migrations/0011_preview_counts.sql"),
        include_str!("../migrations/0012_saved_folders.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        12,
    );
    drop(connection);

    let destination = NativePathKey::from_path(Path::new("/Users/example/Exports"));
    let mut catalog = Catalog::open(&path).unwrap();
    assert_eq!(catalog.photo_pick_revision().unwrap(), 0);
    assert!(catalog.list_photo_picks().unwrap().is_empty());
    catalog.set_last_copy_destination(&destination).unwrap();
    drop(catalog);

    let catalog = Catalog::open(&path).unwrap();
    assert_eq!(catalog.photo_pick_revision().unwrap(), 0);
    assert!(catalog.list_photo_picks().unwrap().is_empty());
    assert_eq!(catalog.last_copy_destination().unwrap(), Some(destination));
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    assert_eq!(
        connection
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        15,
    );
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM photo_pick_preferences", [], |row| row
                .get::<_, i64>(0),)
            .unwrap(),
        1,
    );
}
