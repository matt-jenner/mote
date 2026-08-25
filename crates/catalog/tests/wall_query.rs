use std::path::Path;

use photo_catalog::{
    AssetMetadataUpdate, AssetShapeUpdate, Catalog, CatalogIndexRecord, NewAsset, NewFolderGroup,
    NewLibrary, ShapeStatus, WallOrder,
};
use photo_domain::{FolderGroupId, MediaKind, RelativePathKey};

fn ready_group(catalog: &mut Catalog, library: photo_domain::LibraryId) -> FolderGroupId {
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library,
            relative_path: RelativePathKey::from_relative_path(Path::new("group")).unwrap(),
            display_path: "group".into(),
            last_viewed_at: None,
        })
        .unwrap()
}

#[test]
fn new_assets_keep_append_only_provisional_order_across_upserts() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
        .unwrap();
    let group = ready_group(&mut catalog, library.id);
    for path in ["b.jpg", "a.jpg", "c.jpg"] {
        let key = RelativePathKey::from_relative_path(Path::new(path)).unwrap();
        let mut asset = NewAsset::minimal(library.id, key, path, MediaKind::Jpeg, 10);
        asset.folder_group_id = Some(group);
        catalog.upsert_asset(&asset).unwrap();
        catalog
            .apply_index_batch(&[CatalogIndexRecord::Shaped(AssetShapeUpdate {
                asset_id: asset.id,
                width: 16,
                height: 9,
                orientation: Some(1),
                representative_rgb: None,
                shape_status: ShapeStatus::Ready,
            })])
            .unwrap();
    }
    let first = catalog
        .wall_page(group, WallOrder::Provisional, None, 10)
        .unwrap();
    assert_eq!(
        first
            .items
            .iter()
            .map(|item| item.display_path.as_str())
            .collect::<Vec<_>>(),
        ["b.jpg", "a.jpg", "c.jpg"]
    );

    let key = RelativePathKey::from_relative_path(Path::new("b.jpg")).unwrap();
    let mut changed = NewAsset::minimal(library.id, key, "b.jpg", MediaKind::Jpeg, 11);
    changed.folder_group_id = Some(group);
    catalog.upsert_asset(&changed).unwrap();
    let second = catalog
        .wall_page(group, WallOrder::Provisional, None, 10)
        .unwrap();
    assert_eq!(
        second
            .items
            .iter()
            .map(|item| item.provisional_order)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
}

struct WallFixture {
    catalog: Catalog,
    library: photo_domain::LibraryId,
    group: FolderGroupId,
}
impl WallFixture {
    fn new() -> Self {
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = catalog
            .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
            .unwrap();
        let group = ready_group(&mut catalog, library.id);
        Self {
            catalog,
            library: library.id,
            group,
        }
    }
    fn add(
        &mut self,
        path: &str,
        status: Option<ShapeStatus>,
        date: Option<&str>,
        width: u32,
        height: u32,
    ) -> photo_domain::AssetId {
        let key = RelativePathKey::from_relative_path(Path::new(path)).unwrap();
        let mut asset = NewAsset::minimal(self.library, key, path, MediaKind::Jpeg, 1);
        asset.folder_group_id = Some(self.group);
        let id = asset.id;
        self.catalog.upsert_asset(&asset).unwrap();
        if let Some(shape_status) = status {
            self.catalog
                .apply_index_batch(&[CatalogIndexRecord::Shaped(AssetShapeUpdate {
                    asset_id: id,
                    width,
                    height,
                    orientation: Some(1),
                    representative_rgb: None,
                    shape_status,
                })])
                .unwrap();
        }
        if let Some(captured_at_utc) = date {
            self.catalog
                .apply_index_batch(&[CatalogIndexRecord::Metadata(AssetMetadataUpdate {
                    asset_id: id,
                    captured_at_utc: Some(captured_at_utc.into()),
                    rating: None,
                    keywords: vec![],
                    provenance: vec![],
                })])
                .unwrap();
        }
        id
    }
    fn ready(&mut self, path: &str, date: &str) -> photo_domain::AssetId {
        self.add(path, Some(ShapeStatus::Ready), Some(date), 16, 9)
    }
    fn shaped(
        &mut self,
        path: &str,
        status: ShapeStatus,
        width: u32,
        height: u32,
    ) -> photo_domain::AssetId {
        self.add(path, Some(status), None, width, height)
    }
    fn pending(&mut self, path: &str) -> photo_domain::AssetId {
        self.add(path, None, None, 0, 0)
    }
}

#[test]
fn settled_pages_sort_dates_in_both_directions_with_stable_ties() {
    let mut fixture = WallFixture::new();
    let first = fixture.ready("one.jpg", "2024-01-01T00:00:00Z");
    let second = fixture.ready("two.jpg", "2024-01-02T00:00:00Z");
    let third = fixture.ready("three.jpg", "2024-01-03T00:00:00Z");
    let ascending = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedAscending, None, 2)
        .unwrap();
    assert_eq!(
        ascending
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [first, second]
    );
    let tail = fixture
        .catalog
        .wall_page(
            fixture.group,
            WallOrder::CapturedAscending,
            ascending.next,
            2,
        )
        .unwrap();
    assert_eq!(
        tail.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [third]
    );
    let descending = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedDescending, None, 3)
        .unwrap();
    assert_eq!(
        descending
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [third, second, first]
    );
}

#[test]
fn wall_page_excludes_pending_shapes_but_keeps_fallback_shapes() {
    let mut fixture = WallFixture::new();
    let pending = fixture.pending("pending.jpg");
    let ready = fixture.shaped("ready.jpg", ShapeStatus::Ready, 16, 9);
    let fallback = fixture.shaped("broken.jpg", ShapeStatus::Fallback, 4, 3);
    let page = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::Provisional, None, 10)
        .unwrap();
    assert!(!page.items.iter().any(|item| item.id == pending));
    assert_eq!(
        page.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [ready, fallback]
    );
    assert_eq!(
        (
            page.items[1].width,
            page.items[1].height,
            page.items[1].shape_status
        ),
        (4, 3, ShapeStatus::Fallback)
    );
}
