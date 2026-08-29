use std::path::Path;

use photo_catalog::{
    AssetMetadataUpdate, AssetShapeUpdate, Catalog, CatalogError, CatalogIndexRecord, NewAsset,
    NewFolderGroup, NewLibrary, ShapeStatus, WallCursorKey, WallOrder,
};
use photo_domain::{AssetId, FolderGroupId, GalleryScope, MediaKind, RelativePathKey};
use rusqlite::Connection;

fn id_key(id: photo_domain::AssetId) -> [u8; 16] {
    *id.as_uuid().as_bytes()
}

fn ready_group(catalog: &mut Catalog, library: photo_domain::LibraryId) -> FolderGroupId {
    ready_group_at(catalog, library, "group")
}

fn ready_group_at(
    catalog: &mut Catalog,
    library: photo_domain::LibraryId,
    path: &str,
) -> FolderGroupId {
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library,
            relative_path: RelativePathKey::from_relative_path(Path::new(path)).unwrap(),
            display_path: path.into(),
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

#[test]
fn provisional_pages_paginate_with_stable_order() {
    let mut fixture = WallFixture::new();
    let first = fixture.shaped("one.jpg", ShapeStatus::Ready, 1, 1);
    let second = fixture.shaped("two.jpg", ShapeStatus::Ready, 1, 1);
    let third = fixture.shaped("three.jpg", ShapeStatus::Ready, 1, 1);
    let page = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::Provisional, None, 2)
        .unwrap();
    assert_eq!(
        page.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [first, second]
    );
    let tail = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::Provisional, page.next, 2)
        .unwrap();
    assert_eq!(
        tail.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [third]
    );
}

#[test]
fn current_folder_scope_excludes_nested_assets_without_breaking_recursive_pagination() {
    let mut fixture = WallFixture::new();
    fixture.group = ready_group_at(&mut fixture.catalog, fixture.library, "selected");
    let direct = fixture.shaped("selected/direct.jpg", ShapeStatus::Ready, 16, 9);
    let nested = fixture.shaped("selected/child/nested.jpg", ShapeStatus::Ready, 16, 9);

    let current = fixture
        .catalog
        .wall_page_scoped(
            fixture.group,
            GalleryScope::CurrentFolder,
            WallOrder::Provisional,
            None,
            10,
        )
        .unwrap();
    assert_eq!(
        current.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [direct]
    );

    let first_recursive = fixture
        .catalog
        .wall_page_scoped(
            fixture.group,
            GalleryScope::IncludeSubfolders,
            WallOrder::Provisional,
            None,
            1,
        )
        .unwrap();
    assert_eq!(first_recursive.items[0].id, direct);
    let second_recursive = fixture
        .catalog
        .wall_page_scoped(
            fixture.group,
            GalleryScope::IncludeSubfolders,
            WallOrder::Provisional,
            first_recursive.next,
            1,
        )
        .unwrap();
    assert_eq!(second_recursive.items[0].id, nested);
}

#[test]
fn wall_pages_skip_videos_before_limit_and_cursor_calculation() {
    let fixture = WallFixture::with_assets([
        asset("a.jpg", MediaKind::Jpeg, 1),
        asset("clip.mp4", MediaKind::Video, 2),
        asset("b.jpg", MediaKind::Jpeg, 3),
    ]);

    let first = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::Provisional, None, 1)
        .unwrap();
    assert_eq!(display_paths(&first.items), ["a.jpg"]);

    let second = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::Provisional, first.next, 1)
        .unwrap();
    assert_eq!(display_paths(&second.items), ["b.jpg"]);
    assert!(second.next.is_some());
}

#[test]
fn captured_wall_pages_skip_videos_and_preserve_literal_cursors() {
    let fixture = WallFixture::with_assets([
        captured_asset("a.jpg", MediaKind::Jpeg, 1, "2024-01-01T00:00:00Z"),
        captured_asset("clip.mp4", MediaKind::Video, 2, "2024-01-02T00:00:00Z"),
        captured_asset("b.jpg", MediaKind::Jpeg, 3, "2024-01-03T00:00:00Z"),
    ]);
    let a_id = AssetId::for_path(
        fixture.library,
        &RelativePathKey::from_relative_path(Path::new("a.jpg")).unwrap(),
    );
    let b_id = AssetId::for_path(
        fixture.library,
        &RelativePathKey::from_relative_path(Path::new("b.jpg")).unwrap(),
    );

    let ascending = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedAscending, None, 1)
        .unwrap();
    assert_eq!(display_paths(&ascending.items), ["a.jpg"]);
    assert_eq!(
        ascending.next,
        Some(WallCursorKey::Captured {
            captured_at_utc: "2024-01-01T00:00:00Z".to_owned(),
            display_path: "a.jpg".to_owned(),
            id: a_id,
        })
    );
    let ascending_tail = fixture
        .catalog
        .wall_page(
            fixture.group,
            WallOrder::CapturedAscending,
            ascending.next,
            1,
        )
        .unwrap();
    assert_eq!(display_paths(&ascending_tail.items), ["b.jpg"]);
    assert_eq!(
        ascending_tail.next,
        Some(WallCursorKey::Captured {
            captured_at_utc: "2024-01-03T00:00:00Z".to_owned(),
            display_path: "b.jpg".to_owned(),
            id: b_id,
        })
    );

    let descending = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedDescending, None, 1)
        .unwrap();
    assert_eq!(display_paths(&descending.items), ["b.jpg"]);
    assert_eq!(
        descending.next,
        Some(WallCursorKey::Captured {
            captured_at_utc: "2024-01-03T00:00:00Z".to_owned(),
            display_path: "b.jpg".to_owned(),
            id: b_id,
        })
    );
    let descending_tail = fixture
        .catalog
        .wall_page(
            fixture.group,
            WallOrder::CapturedDescending,
            descending.next,
            1,
        )
        .unwrap();
    assert_eq!(display_paths(&descending_tail.items), ["a.jpg"]);
    assert_eq!(
        descending_tail.next,
        Some(WallCursorKey::Captured {
            captured_at_utc: "2024-01-01T00:00:00Z".to_owned(),
            display_path: "a.jpg".to_owned(),
            id: a_id,
        })
    );
}

#[test]
fn wall_records_for_assets_omit_video_ids() {
    let fixture = WallFixture::with_photo_and_video();
    let rows = fixture
        .catalog
        .wall_records_for_assets(fixture.group, &[fixture.photo, fixture.video])
        .unwrap();
    assert_eq!(
        rows.iter().map(|row| row.id).collect::<Vec<_>>(),
        [fixture.photo]
    );
}

#[test]
fn scoped_wall_records_and_prefetch_ids_exclude_nested_assets() {
    let mut fixture = WallFixture::new();
    fixture.group = ready_group_at(&mut fixture.catalog, fixture.library, "selected");
    let direct = fixture.shaped("selected/direct.jpg", ShapeStatus::Ready, 16, 9);
    let nested = fixture.shaped("selected/child/nested.jpg", ShapeStatus::Ready, 16, 9);

    let records = fixture
        .catalog
        .wall_records_for_assets_scoped(
            fixture.group,
            GalleryScope::CurrentFolder,
            &[direct, nested],
        )
        .unwrap();
    assert_eq!(
        records.iter().map(|record| record.id).collect::<Vec<_>>(),
        [direct]
    );

    let ids = fixture
        .catalog
        .photo_asset_ids_page_scoped(
            fixture.group,
            GalleryScope::CurrentFolder,
            WallOrder::Provisional,
            None,
            10,
        )
        .unwrap();
    assert_eq!(ids.items, [direct]);
}

#[test]
fn photo_asset_ids_page_projects_photo_ids_with_keyset_cursor() {
    let fixture = WallFixture::with_assets([
        asset("a.jpg", MediaKind::Jpeg, 1),
        asset("clip.mp4", MediaKind::Video, 2),
        asset("b.jpg", MediaKind::Jpeg, 3),
    ]);
    let first = fixture
        .catalog
        .photo_asset_ids_page(fixture.group, WallOrder::Provisional, None, 1)
        .unwrap();
    let first_id = AssetId::for_path(
        fixture.library,
        &RelativePathKey::from_relative_path(Path::new("a.jpg")).unwrap(),
    );
    assert_eq!(first.items, [first_id]);

    let second = fixture
        .catalog
        .photo_asset_ids_page(fixture.group, WallOrder::Provisional, first.next, 1)
        .unwrap();
    let second_id = AssetId::for_path(
        fixture.library,
        &RelativePathKey::from_relative_path(Path::new("b.jpg")).unwrap(),
    );
    assert_eq!(second.items, [second_id]);
    assert!(second.next.is_some());
}

#[test]
fn wall_page_projects_asset_rating() {
    let mut fixture = WallFixture::new();
    let id = fixture.shaped("rated.jpg", ShapeStatus::Ready, 16, 9);
    fixture
        .catalog
        .apply_index_batch(&[CatalogIndexRecord::Metadata(AssetMetadataUpdate {
            asset_id: id,
            captured_at_utc: None,
            rating: Some(4),
            keywords: vec![],
            provenance: vec![],
        })])
        .unwrap();

    let page = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::Provisional, None, 10)
        .unwrap();
    assert_eq!(page.items[0].rating, Some(4));
}

struct WallFixture {
    catalog: Catalog,
    library: photo_domain::LibraryId,
    group: FolderGroupId,
    photo: AssetId,
    video: AssetId,
}

#[derive(Clone, Copy)]
struct FixtureAsset {
    path: &'static str,
    media_kind: MediaKind,
    order: u64,
    captured_at_utc: Option<&'static str>,
}

fn asset(path: &'static str, media_kind: MediaKind, order: u64) -> FixtureAsset {
    FixtureAsset {
        path,
        media_kind,
        order,
        captured_at_utc: None,
    }
}

fn captured_asset(
    path: &'static str,
    media_kind: MediaKind,
    order: u64,
    captured_at_utc: &'static str,
) -> FixtureAsset {
    FixtureAsset {
        path,
        media_kind,
        order,
        captured_at_utc: Some(captured_at_utc),
    }
}

fn display_paths(items: &[photo_catalog::WallCatalogRecord]) -> Vec<&str> {
    items
        .iter()
        .map(|item| item.display_path.as_str())
        .collect()
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
            photo: AssetId::from_uuid(uuid::Uuid::nil()),
            video: AssetId::from_uuid(uuid::Uuid::from_bytes([1; 16])),
        }
    }

    fn with_assets<const N: usize>(assets: [FixtureAsset; N]) -> Self {
        let mut fixture = Self::new();
        let mut assets = assets;
        assets.sort_by_key(|asset| asset.order);
        for item in assets {
            let key = RelativePathKey::from_relative_path(Path::new(item.path)).unwrap();
            let mut value = NewAsset::minimal(fixture.library, key, item.path, item.media_kind, 1);
            value.folder_group_id = Some(fixture.group);
            fixture.catalog.upsert_asset(&value).unwrap();
            fixture
                .catalog
                .apply_index_batch(&[
                    CatalogIndexRecord::Shaped(AssetShapeUpdate {
                        asset_id: value.id,
                        width: 16,
                        height: 9,
                        orientation: Some(1),
                        representative_rgb: None,
                        shape_status: ShapeStatus::Ready,
                    }),
                    CatalogIndexRecord::Metadata(AssetMetadataUpdate {
                        asset_id: value.id,
                        captured_at_utc: item.captured_at_utc.map(Into::into),
                        rating: None,
                        keywords: vec![],
                        provenance: vec![],
                    }),
                ])
                .unwrap();
        }
        fixture
    }

    fn with_photo_and_video() -> Self {
        let mut fixture = Self::with_assets([
            asset("photo.jpg", MediaKind::Jpeg, 1),
            asset("clip.mp4", MediaKind::Video, 2),
        ]);
        fixture.photo = fixture
            .catalog
            .find_asset(AssetId::for_path(
                fixture.library,
                &RelativePathKey::from_relative_path(Path::new("photo.jpg")).unwrap(),
            ))
            .unwrap()
            .unwrap()
            .id;
        fixture.video = fixture
            .catalog
            .find_asset(AssetId::for_path(
                fixture.library,
                &RelativePathKey::from_relative_path(Path::new("clip.mp4")).unwrap(),
            ))
            .unwrap()
            .unwrap()
            .id;
        fixture
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
    fn add_with_display(
        &mut self,
        relative: &str,
        display: &str,
        status: ShapeStatus,
        date: &str,
    ) -> photo_domain::AssetId {
        let key = RelativePathKey::from_relative_path(Path::new(relative)).unwrap();
        let mut asset = NewAsset::minimal(self.library, key, display, MediaKind::Jpeg, 1);
        asset.folder_group_id = Some(self.group);
        let id = asset.id;
        self.catalog.upsert_asset(&asset).unwrap();
        self.catalog
            .apply_index_batch(&[
                CatalogIndexRecord::Shaped(AssetShapeUpdate {
                    asset_id: id,
                    width: 16,
                    height: 9,
                    orientation: Some(1),
                    representative_rgb: None,
                    shape_status: status,
                }),
                CatalogIndexRecord::Metadata(AssetMetadataUpdate {
                    asset_id: id,
                    captured_at_utc: Some(date.into()),
                    rating: None,
                    keywords: vec![],
                    provenance: vec![],
                }),
            ])
            .unwrap();
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

    fn new_asset(&self, path: &str, group: FolderGroupId) -> NewAsset {
        let key = RelativePathKey::from_relative_path(Path::new(path)).unwrap();
        let mut asset = NewAsset::minimal(self.library, key, path, MediaKind::Jpeg, 1);
        asset.folder_group_id = Some(group);
        asset
    }

    fn asset_in_group(&mut self, path: &str, group: FolderGroupId) -> photo_domain::AssetId {
        let asset = self.new_asset(path, group);
        let id = asset.id;
        self.catalog.upsert_asset(&asset).unwrap();
        id
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
fn settled_pages_use_path_ties_in_both_directions() {
    let mut fixture = WallFixture::new();
    let first = fixture.ready("a.jpg", "2024-01-01T00:00:00Z");
    let second = fixture.ready("b.jpg", "2024-01-01T00:00:00Z");
    let ascending = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedAscending, None, 1)
        .unwrap();
    assert_eq!(ascending.items[0].id, first);
    let ascending_tail = fixture
        .catalog
        .wall_page(
            fixture.group,
            WallOrder::CapturedAscending,
            ascending.next,
            1,
        )
        .unwrap();
    assert_eq!(ascending_tail.items[0].id, second);
    let descending = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedDescending, None, 2)
        .unwrap();
    assert_eq!(
        descending
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [first, second]
    );
}

#[test]
fn descending_equal_date_path_ties_paginate_without_skip_or_duplication() {
    let mut fixture = WallFixture::new();
    let first = fixture.ready("a.jpg", "2024-01-01T00:00:00Z");
    let second = fixture.ready("b.jpg", "2024-01-01T00:00:00Z");
    let third = fixture.ready("c.jpg", "2024-01-01T00:00:00Z");
    let page = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedDescending, None, 2)
        .unwrap();
    assert_eq!(
        page.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [first, second]
    );
    let tail = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedDescending, page.next, 2)
        .unwrap();
    assert_eq!(
        tail.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [third]
    );
}

#[test]
fn equal_date_and_display_path_ties_use_ids_in_both_directions() {
    let mut fixture = WallFixture::new();
    let first = fixture.add_with_display(
        "one.jpg",
        "same.jpg",
        ShapeStatus::Ready,
        "2024-01-01T00:00:00Z",
    );
    let second = fixture.add_with_display(
        "two.jpg",
        "same.jpg",
        ShapeStatus::Ready,
        "2024-01-01T00:00:00Z",
    );
    let ascending = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedAscending, None, 1)
        .unwrap();
    let ascending_tail = fixture
        .catalog
        .wall_page(
            fixture.group,
            WallOrder::CapturedAscending,
            ascending.next,
            1,
        )
        .unwrap();
    assert_eq!(
        id_key(ascending.items[0].id),
        [id_key(first), id_key(second)].into_iter().min().unwrap()
    );
    assert_ne!(ascending.items[0].id, ascending_tail.items[0].id);
    assert_eq!(
        id_key(ascending_tail.items[0].id),
        [id_key(first), id_key(second)].into_iter().max().unwrap()
    );
    let descending = fixture
        .catalog
        .wall_page(fixture.group, WallOrder::CapturedDescending, None, 1)
        .unwrap();
    let descending_tail = fixture
        .catalog
        .wall_page(
            fixture.group,
            WallOrder::CapturedDescending,
            descending.next,
            1,
        )
        .unwrap();
    assert_eq!(
        id_key(descending.items[0].id),
        [id_key(first), id_key(second)].into_iter().min().unwrap()
    );
    assert_ne!(descending.items[0].id, descending_tail.items[0].id);
    assert_eq!(
        id_key(descending_tail.items[0].id),
        [id_key(first), id_key(second)].into_iter().max().unwrap()
    );
}

#[test]
fn incomplete_generations_report_false() {
    let mut fixture = WallFixture::new();
    let generation = fixture.catalog.begin_generation(fixture.library).unwrap();
    assert!(
        !fixture
            .catalog
            .has_completed_generation(fixture.library, generation)
            .unwrap()
    );
    fixture
        .catalog
        .complete_generation(fixture.library, generation)
        .unwrap();
    assert!(
        fixture
            .catalog
            .has_completed_generation(fixture.library, generation)
            .unwrap()
    );
}

#[test]
fn completing_a_group_generation_only_marks_missing_assets_in_that_group() {
    let mut fixture = WallFixture::new();
    let sibling = ready_group_at(&mut fixture.catalog, fixture.library, "sibling");
    let retained = fixture.asset_in_group("retained.jpg", fixture.group);
    let missing = fixture.asset_in_group("missing.jpg", fixture.group);
    let sibling_asset = fixture.asset_in_group("sibling.jpg", sibling);

    let generation = fixture
        .catalog
        .begin_generation_for_group(fixture.library, fixture.group)
        .unwrap();
    fixture
        .catalog
        .record_generation_assets_for_group(
            fixture.library,
            fixture.group,
            generation,
            &[fixture.new_asset("retained.jpg", fixture.group)],
        )
        .unwrap();
    let completion = fixture
        .catalog
        .complete_generation_for_group(fixture.library, fixture.group, generation)
        .unwrap();

    assert_eq!(completion.marked_missing, 1);
    assert_eq!(
        fixture
            .catalog
            .find_asset(retained)
            .unwrap()
            .unwrap()
            .availability,
        photo_domain::Availability::Available
    );
    assert_eq!(
        fixture
            .catalog
            .find_asset(missing)
            .unwrap()
            .unwrap()
            .availability,
        photo_domain::Availability::Missing
    );
    assert_eq!(
        fixture
            .catalog
            .find_asset(sibling_asset)
            .unwrap()
            .unwrap()
            .availability,
        photo_domain::Availability::Available
    );
}

#[test]
fn completed_generation_is_scoped_to_its_folder_group() {
    let mut fixture = WallFixture::new();
    let sibling = ready_group_at(&mut fixture.catalog, fixture.library, "sibling");
    let generation = fixture
        .catalog
        .begin_generation_for_group(fixture.library, fixture.group)
        .unwrap();
    fixture
        .catalog
        .complete_generation_for_group(fixture.library, fixture.group, generation)
        .unwrap();

    assert!(
        fixture
            .catalog
            .has_completed_group_generation(fixture.library, fixture.group, generation)
            .unwrap()
    );
    assert!(
        !fixture
            .catalog
            .has_completed_generation_for_group(fixture.library, sibling)
            .unwrap()
    );
}

#[test]
fn opening_a_v4_catalog_adds_group_generation_support() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let connection = Connection::open(&path).unwrap();
    for migration in [
        include_str!("../migrations/0001_catalog.sql"),
        include_str!("../migrations/0002_unavailable_assets.sql"),
        include_str!("../migrations/0003_app_state.sql"),
        include_str!("../migrations/0004_wall_projection.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }
    drop(connection);

    let mut catalog = Catalog::open(&path).unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
        .unwrap();
    let group = ready_group(&mut catalog, library.id);
    let generation = catalog
        .begin_generation_for_group(library.id, group)
        .unwrap();
    assert!(
        !catalog
            .has_completed_group_generation(library.id, group, generation)
            .unwrap()
    );
}

#[test]
fn opening_a_populated_v4_catalog_backfills_settled_groups_offline() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let connection = Connection::open(&path).unwrap();
    for migration in [
        include_str!("../migrations/0001_catalog.sql"),
        include_str!("../migrations/0002_unavailable_assets.sql"),
        include_str!("../migrations/0003_app_state.sql"),
        include_str!("../migrations/0004_wall_projection.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }

    let library = [1_u8; 16];
    let first_group = [2_u8; 16];
    let second_group = [3_u8; 16];
    let first_asset = [4_u8; 16];
    let second_asset = [5_u8; 16];
    let derivative = [6_u8; 16];
    let first_older_asset = [7_u8; 16];
    let second_newer_asset = [8_u8; 16];
    connection
        .execute(
            "INSERT INTO library_roots
                (id, kind, display_name, canonical_root_key, display_path, availability)
             VALUES (?1, 'configured', 'Photos', ?1, '/Photos', 'root_offline')",
            [library.as_slice()],
        )
        .unwrap();
    for (id, path) in [(first_group, "first"), (second_group, "second")] {
        let relative = RelativePathKey::from_relative_path(Path::new(path)).unwrap();
        connection
            .execute(
                "INSERT INTO folder_groups
                    (id, library_id, relative_path_key, display_path)
                 VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id.as_slice(), library.as_slice(), relative.as_bytes(), path],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO scan_generations
                (library_id, generation, started_at, completed_at, source_was_online)
             VALUES (?1, 7, 100, 200, 1)",
            [library.as_slice()],
        )
        .unwrap();
    for (provisional, id, group, path, captured) in [
        (
            1_i64,
            first_asset,
            first_group,
            "first/new.jpg",
            "2024-01-02T00:00:00Z",
        ),
        (
            2_i64,
            first_older_asset,
            first_group,
            "first/old.jpg",
            "2024-01-01T00:00:00Z",
        ),
        (
            3_i64,
            second_asset,
            second_group,
            "second/old.jpg",
            "2024-01-01T00:00:00Z",
        ),
        (
            4_i64,
            second_newer_asset,
            second_group,
            "second/new.jpg",
            "2024-01-03T00:00:00Z",
        ),
    ] {
        let relative = RelativePathKey::from_relative_path(Path::new(path)).unwrap();
        connection
            .execute(
                "INSERT INTO assets
                    (id, library_id, folder_group_id, relative_path_key, display_path,
                     media_kind, size_bytes, modified_unix_ns, width, height, availability,
                     captured_at_utc, last_seen_generation, provisional_order, shape_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, 'jpeg', 10, '10', 1600, 900,
                         'root_offline', ?6, 7, ?7, 'ready')",
                rusqlite::params![
                    id.as_slice(),
                    library.as_slice(),
                    group.as_slice(),
                    relative.as_bytes(),
                    path,
                    captured,
                    provisional,
                ],
            )
            .unwrap();
    }
    connection
        .execute(
            "INSERT INTO derivatives
                (id, asset_id, folder_group_id, kind, cache_key, relative_cache_path,
                 size_bytes, durable, created_at)
             VALUES (?1, ?2, ?3, 'wall_thumbnail', 'cached-wall', 'wall/cached.jpg', 12, 1, 200)",
            rusqlite::params![
                derivative.as_slice(),
                first_asset.as_slice(),
                first_group.as_slice()
            ],
        )
        .unwrap();
    drop(connection);

    let catalog = Catalog::open(&path).unwrap();
    assert!(
        catalog
            .has_completed_generation_for_group(
                photo_domain::LibraryId::from_uuid(uuid::Uuid::from_bytes(library)),
                FolderGroupId::from_uuid(uuid::Uuid::from_bytes(first_group)),
            )
            .unwrap()
    );
    assert!(
        catalog
            .has_completed_generation_for_group(
                photo_domain::LibraryId::from_uuid(uuid::Uuid::from_bytes(library)),
                FolderGroupId::from_uuid(uuid::Uuid::from_bytes(second_group)),
            )
            .unwrap()
    );

    let first_page = catalog
        .wall_page(
            FolderGroupId::from_uuid(uuid::Uuid::from_bytes(first_group)),
            WallOrder::CapturedAscending,
            None,
            10,
        )
        .unwrap();
    assert_eq!(first_page.items.len(), 2);
    assert_eq!(first_page.items[0].display_path, "first/old.jpg");
    assert_eq!(first_page.items[1].display_path, "first/new.jpg");
    assert_eq!(
        first_page.items[0].availability,
        photo_domain::Availability::RootOffline
    );

    let second_page = catalog
        .wall_page(
            FolderGroupId::from_uuid(uuid::Uuid::from_bytes(second_group)),
            WallOrder::CapturedAscending,
            None,
            10,
        )
        .unwrap();
    assert_eq!(second_page.items.len(), 2);
    assert_eq!(second_page.items[0].display_path, "second/old.jpg");
    assert_eq!(second_page.items[1].display_path, "second/new.jpg");
    assert_eq!(catalog.all_derivatives().unwrap().len(), 1);
}

#[test]
fn wall_page_excludes_pending_shapes_but_keeps_fallback_shapes() {
    let mut fixture = WallFixture::new();
    let pending = fixture.pending("pending.jpg");
    assert_eq!(
        fixture
            .catalog
            .find_asset(pending)
            .unwrap()
            .unwrap()
            .shape_status,
        ShapeStatus::Pending
    );
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

#[test]
fn wall_page_rejects_a_cursor_variant_that_does_not_match_the_order() {
    let mut fixture = WallFixture::new();
    fixture.shaped("photo.jpg", ShapeStatus::Ready, 16, 9);

    let error = fixture
        .catalog
        .wall_page(
            fixture.group,
            WallOrder::Provisional,
            Some(WallCursorKey::Captured {
                captured_at_utc: "2024-01-01T00:00:00Z".to_owned(),
                display_path: "photo.jpg".to_owned(),
                id: photo_domain::AssetId::from_uuid(uuid::Uuid::from_bytes([99_u8; 16])),
            }),
            1,
        )
        .unwrap_err();

    assert!(matches!(error, CatalogError::WallCursorOrderMismatch));
}

#[test]
fn opening_a_populated_v3_catalog_upgrades_shapes_and_normalizes_legacy_dates_offline() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let connection = Connection::open(&path).unwrap();
    for migration in [
        include_str!("../migrations/0001_catalog.sql"),
        include_str!("../migrations/0002_unavailable_assets.sql"),
        include_str!("../migrations/0003_app_state.sql"),
    ] {
        connection.execute_batch(migration).unwrap();
    }

    let library = [11_u8; 16];
    let group = [12_u8; 16];
    let older = [13_u8; 16];
    let newer = [14_u8; 16];
    let invalid = [15_u8; 16];
    connection
        .execute(
            "INSERT INTO library_roots
                (id, kind, display_name, canonical_root_key, display_path, availability)
             VALUES (?1, 'configured', 'Pictures', ?1, '/Pictures', 'root_offline')",
            [library.as_slice()],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO folder_groups
                (id, library_id, relative_path_key, display_path)
             VALUES (?1, ?2, ?3, 'selected')",
            rusqlite::params![
                group.as_slice(),
                library.as_slice(),
                b"Uselected".as_slice()
            ],
        )
        .unwrap();
    connection
        .execute(
            "INSERT INTO assets
                (id, library_id, folder_group_id, relative_path_key, display_path,
                 media_kind, size_bytes, modified_unix_ns, width, height, availability,
                 captured_at_utc)
             VALUES (?1, ?2, ?3, ?4, 'selected/newer.jpg', 'jpeg', 10, '10', 1600, 900,
                     'root_offline', '2024-01-01T00:30:00+02:00'),
                    (?5, ?2, ?3, ?6, 'selected/older.jpg', 'jpeg', 10, '10', 800, 600,
                    'root_offline', '2023-12-31T22:30:00+01:00'),
                    (?7, ?2, ?3, ?8, 'selected/invalid.jpg', 'jpeg', 10, '10', 640, 480,
                     'root_offline', 'not-a-timestamp')",
            rusqlite::params![
                newer.as_slice(),
                library.as_slice(),
                group.as_slice(),
                b"Uselected/newer.jpg".as_slice(),
                older.as_slice(),
                b"Uselected/older.jpg".as_slice(),
                invalid.as_slice(),
                b"Uselected/invalid.jpg".as_slice(),
            ],
        )
        .unwrap();
    drop(connection);

    let catalog = Catalog::open(&path).unwrap();
    let group_id = FolderGroupId::from_uuid(uuid::Uuid::from_bytes(group));
    let page = catalog
        .wall_page(group_id, WallOrder::CapturedAscending, None, 10)
        .unwrap();

    assert_eq!(page.items.len(), 3);
    assert_eq!(page.items[0].display_path, "selected/older.jpg");
    assert_eq!(page.items[1].display_path, "selected/newer.jpg");
    assert_eq!(page.items[2].display_path, "selected/invalid.jpg");
    assert!(
        page.items
            .iter()
            .all(|item| item.shape_status == ShapeStatus::Ready)
    );
    assert_eq!(
        page.items[0].captured_at_utc.as_deref(),
        Some("2023-12-31T21:30:00+00:00")
    );
    assert_eq!(
        page.items[1].captured_at_utc.as_deref(),
        Some("2023-12-31T22:30:00+00:00")
    );
    assert_eq!(
        catalog
            .find_asset(photo_domain::AssetId::from_uuid(uuid::Uuid::from_bytes(
                invalid
            )))
            .unwrap()
            .unwrap()
            .captured_at_utc
            .as_deref(),
        Some("not-a-timestamp")
    );
    assert!(
        page.items
            .iter()
            .all(|item| { item.availability == photo_domain::Availability::RootOffline })
    );
    let current_folder = catalog
        .wall_page_scoped(
            group_id,
            GalleryScope::CurrentFolder,
            WallOrder::CapturedAscending,
            None,
            10,
        )
        .unwrap();
    assert_eq!(current_folder.items.len(), 3);
}
