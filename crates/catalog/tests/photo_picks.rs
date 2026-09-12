use std::path::Path;

use photo_catalog::{Catalog, NewAsset, NewFolderGroup, NewLibrary};
use photo_domain::{AssetId, FolderGroupId, MediaKind, NativePathKey, RelativePathKey};

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
