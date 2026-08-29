use std::path::Path;

use photo_catalog::{
    AssetShapeUpdate, Catalog, CatalogIndexRecord, NewAsset, NewDerivative, NewFolderGroup,
    NewLibrary, ShapeStatus, WallOrder,
};
use photo_domain::{
    AssetId, DerivativeId, FolderGroupId, GalleryScope, MediaKind, RelativePathKey,
};

fn group(catalog: &mut Catalog, library: photo_domain::LibraryId, path: &str) -> FolderGroupId {
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library,
            relative_path: RelativePathKey::from_relative_path(Path::new(path)).unwrap(),
            display_path: path.to_owned(),
            last_viewed_at: None,
        })
        .unwrap()
}

fn ready_asset(
    catalog: &mut Catalog,
    library: photo_domain::LibraryId,
    group: FolderGroupId,
    path: &str,
) -> AssetId {
    let key = RelativePathKey::from_relative_path(Path::new(path)).unwrap();
    let asset = NewAsset {
        folder_group_id: Some(group),
        ..NewAsset::minimal(library, key, path, MediaKind::Jpeg, 1)
    };
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
    asset.id
}

#[test]
fn parent_and_child_memberships_are_independent_scopes_and_generations() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
        .unwrap();
    let parent = group(&mut catalog, library.id, "selected");
    let child = group(&mut catalog, library.id, "selected/child");
    let direct = ready_asset(&mut catalog, library.id, parent, "selected/direct.jpg");
    let nested = ready_asset(&mut catalog, library.id, child, "selected/child/nested.jpg");

    let parent_generation = catalog
        .begin_generation_for_group(library.id, parent)
        .unwrap();
    catalog
        .add_asset_membership(parent, direct, parent_generation)
        .unwrap();
    catalog
        .add_asset_membership(parent, nested, parent_generation)
        .unwrap();
    let child_generation = catalog
        .begin_generation_for_group(library.id, child)
        .unwrap();
    catalog
        .add_asset_membership(child, nested, child_generation)
        .unwrap();

    assert_eq!(
        catalog
            .wall_page_scoped(
                parent,
                GalleryScope::CurrentFolder,
                WallOrder::Provisional,
                None,
                10,
            )
            .unwrap()
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![direct]
    );
    assert_eq!(
        catalog
            .wall_page_scoped(
                parent,
                GalleryScope::IncludeSubfolders,
                WallOrder::Provisional,
                None,
                10,
            )
            .unwrap()
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![direct, nested]
    );
    for scope in [GalleryScope::CurrentFolder, GalleryScope::IncludeSubfolders] {
        assert_eq!(
            catalog
                .wall_page_scoped(child, scope, WallOrder::Provisional, None, 10)
                .unwrap()
                .items
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![nested]
        );
    }

    let next_parent_generation = catalog
        .begin_generation_for_group(library.id, parent)
        .unwrap();
    catalog
        .add_asset_membership(parent, direct, next_parent_generation)
        .unwrap();
    catalog
        .finish_group_generation(library.id, parent, next_parent_generation)
        .unwrap();
    assert_eq!(
        catalog
            .wall_page(parent, WallOrder::Provisional, None, 10)
            .unwrap()
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![direct]
    );
    assert_eq!(
        catalog
            .wall_page(child, WallOrder::Provisional, None, 10)
            .unwrap()
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        vec![nested]
    );
}

#[test]
fn one_immutable_derivative_can_be_reused_by_multiple_groups() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
        .unwrap();
    let parent = group(&mut catalog, library.id, "selected");
    let child = group(&mut catalog, library.id, "selected/child");
    let asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("selected/photo.jpg")).unwrap(),
        "selected/photo.jpg",
        MediaKind::Jpeg,
        1,
    );
    catalog.upsert_asset(&asset).unwrap();
    let first = NewDerivative {
        id: DerivativeId::new(),
        asset_id: asset.id,
        folder_group_id: parent,
        kind: "screen_preview".to_owned(),
        cache_key: "immutable-preview".to_owned(),
        relative_cache_path: Path::new("ab/cd.jpg").to_owned(),
        size_bytes: 8,
        durable: false,
        created_at: 1,
    };
    catalog.insert_derivative(&first).unwrap();
    catalog
        .insert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            folder_group_id: child,
            ..first.clone()
        })
        .unwrap();

    let stored = catalog
        .find_derivative_by_cache_key("immutable-preview")
        .unwrap()
        .unwrap();
    assert_eq!(stored.id, first.id);
    assert_eq!(catalog.all_derivatives().unwrap().len(), 1);
    assert_eq!(catalog.derivative_count(parent, false).unwrap(), 1);
    assert_eq!(catalog.derivative_count(child, false).unwrap(), 1);
}
