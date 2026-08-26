use std::path::Path;

use photo_catalog::{
    Catalog, CatalogWarningRecord, NewAsset, NewDerivative, NewFolderGroup, NewLibrary,
    SqliteVersion,
};
use photo_domain::{Availability, DerivativeId, FolderGroupId, MediaKind, RelativePathKey};

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

#[test]
fn unavailable_asset_count_tracks_retained_offline_rows() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured(
            "Pictures",
            Path::new("/mounted/Pictures"),
        ))
        .unwrap();
    for display_path in ["a.jpg", "b.jpg", "c.jpg"] {
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

    assert_eq!(catalog.unavailable_asset_count(library.id).unwrap(), 0);
    assert_eq!(catalog.mark_root_offline(library.id).unwrap(), 3);
    assert_eq!(catalog.unavailable_asset_count(library.id).unwrap(), 3);
}

#[test]
fn aggregate_non_durable_cache_bytes_is_zero_and_sums_only_screen_previews() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured(
            "Pictures",
            Path::new("/mounted/Pictures"),
        ))
        .unwrap();
    let group = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(Path::new("selected")).unwrap(),
            display_path: "selected".to_owned(),
            last_viewed_at: None,
        })
        .unwrap();

    assert_eq!(catalog.non_durable_size_bytes().unwrap(), 0);
    let durable_asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("selected/durable.jpg")).unwrap(),
        "selected/durable.jpg",
        MediaKind::Jpeg,
        1,
    );
    let screen_asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("selected/screen.jpg")).unwrap(),
        "selected/screen.jpg",
        MediaKind::Jpeg,
        2,
    );
    catalog.upsert_asset(&durable_asset).unwrap();
    catalog.upsert_asset(&screen_asset).unwrap();
    catalog
        .insert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id: durable_asset.id,
            folder_group_id: group,
            kind: "wall_thumbnail".to_owned(),
            cache_key: "durable-key".to_owned(),
            relative_cache_path: Path::new("durable.jpg").to_owned(),
            size_bytes: 17,
            durable: true,
            created_at: 0,
        })
        .unwrap();
    catalog
        .insert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id: screen_asset.id,
            folder_group_id: group,
            kind: "screen_preview".to_owned(),
            cache_key: "screen-key".to_owned(),
            relative_cache_path: Path::new("screen.jpg").to_owned(),
            size_bytes: 25,
            durable: false,
            created_at: 0,
        })
        .unwrap();

    assert_eq!(catalog.non_durable_size_bytes().unwrap(), 25);
}

#[test]
fn aggregate_non_durable_cache_bytes_rejects_sqlite_sum_overflow() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured(
            "Pictures",
            Path::new("/mounted/Pictures"),
        ))
        .unwrap();
    let group = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(Path::new("selected")).unwrap(),
            display_path: "selected".to_owned(),
            last_viewed_at: None,
        })
        .unwrap();
    let left_asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("selected/left.jpg")).unwrap(),
        "selected/left.jpg",
        MediaKind::Jpeg,
        1,
    );
    let right_asset = NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("selected/right.jpg")).unwrap(),
        "selected/right.jpg",
        MediaKind::Jpeg,
        2,
    );
    catalog.upsert_asset(&left_asset).unwrap();
    catalog.upsert_asset(&right_asset).unwrap();
    for (key, asset_id) in [("left", left_asset.id), ("right", right_asset.id)] {
        catalog
            .insert_derivative(&NewDerivative {
                id: DerivativeId::new(),
                asset_id,
                folder_group_id: group,
                kind: "screen_preview".to_owned(),
                cache_key: key.to_owned(),
                relative_cache_path: Path::new("screen.jpg").to_owned(),
                size_bytes: i64::MAX as u64,
                durable: false,
                created_at: 0,
            })
            .unwrap();
    }

    assert!(matches!(
        catalog.non_durable_size_bytes(),
        Err(photo_catalog::CatalogError::ValueOutOfRange)
    ));
}

#[test]
fn source_warning_snapshot_exposes_only_path_free_codes() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured(
            "Pictures",
            Path::new("/mounted/Pictures"),
        ))
        .unwrap();
    catalog
        .record_warning_once(&CatalogWarningRecord {
            library_id: library.id,
            asset_id: None,
            code: "screen_preview_cache_unavailable".to_owned(),
            message: "/private/source/path must never cross the boundary".to_owned(),
        })
        .unwrap();
    let warnings = catalog.source_warning_summaries(library.id).unwrap();
    assert_eq!(
        warnings
            .iter()
            .map(|warning| warning.code.as_str())
            .collect::<Vec<_>>(),
        vec!["screen_preview_cache_unavailable"]
    );
    assert!(!format!("{warnings:?}").contains("/private/source/path"));
}

#[test]
fn marking_a_folder_group_offline_does_not_mark_sibling_groups_or_root_offline() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured(
            "Pictures",
            Path::new("/mounted/Pictures"),
        ))
        .unwrap();
    let selected = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(Path::new("selected")).unwrap(),
            display_path: "selected".to_owned(),
            last_viewed_at: None,
        })
        .unwrap();
    let sibling = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(Path::new("sibling")).unwrap(),
            display_path: "sibling".to_owned(),
            last_viewed_at: None,
        })
        .unwrap();
    let selected_asset = NewAsset {
        folder_group_id: Some(selected),
        ..NewAsset::minimal(
            library.id,
            RelativePathKey::from_relative_path(Path::new("selected/photo.jpg")).unwrap(),
            "selected/photo.jpg",
            MediaKind::Jpeg,
            1,
        )
    };
    let sibling_asset = NewAsset {
        folder_group_id: Some(sibling),
        ..NewAsset::minimal(
            library.id,
            RelativePathKey::from_relative_path(Path::new("sibling/photo.jpg")).unwrap(),
            "sibling/photo.jpg",
            MediaKind::Jpeg,
            1,
        )
    };
    catalog.upsert_asset(&selected_asset).unwrap();
    catalog.upsert_asset(&sibling_asset).unwrap();

    assert_eq!(catalog.mark_group_offline(library.id, selected).unwrap(), 1);
    assert_eq!(
        catalog
            .find_library(library.id)
            .unwrap()
            .unwrap()
            .availability,
        Availability::Available
    );
    assert_eq!(
        catalog
            .find_asset(selected_asset.id)
            .unwrap()
            .unwrap()
            .availability,
        Availability::RootOffline
    );
    assert_eq!(
        catalog
            .find_asset(sibling_asset.id)
            .unwrap()
            .unwrap()
            .availability,
        Availability::Available
    );
}

#[test]
fn repeated_folder_group_upsert_returns_the_stored_id() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured(
            "Pictures",
            Path::new("/mounted/Pictures"),
        ))
        .unwrap();
    let path = RelativePathKey::from_relative_path(Path::new("2026/Trip")).unwrap();
    let first = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: path.clone(),
            display_path: "2026/Trip".to_owned(),
            last_viewed_at: Some(1),
        })
        .unwrap();

    let second = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: path,
            display_path: "Trip".to_owned(),
            last_viewed_at: Some(2),
        })
        .unwrap();

    assert_eq!(second, first);
}

#[test]
fn reopening_and_touching_a_folder_group_preserves_and_refreshes_recency() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured(
            "Pictures",
            Path::new("/mounted/Pictures"),
        ))
        .unwrap();
    let path = RelativePathKey::from_relative_path(Path::new("2026/Trip")).unwrap();
    let group = catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: path.clone(),
            display_path: "Trip".to_owned(),
            last_viewed_at: Some(10),
        })
        .unwrap();
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id: library.id,
            relative_path: path,
            display_path: "Trip".to_owned(),
            last_viewed_at: None,
        })
        .unwrap();

    assert_eq!(
        catalog.folder_group_last_viewed_at(group).unwrap(),
        Some(10)
    );
    catalog.touch_folder_group(group, 20).unwrap();
    assert_eq!(
        catalog.folder_group_last_viewed_at(group).unwrap(),
        Some(20)
    );
}
