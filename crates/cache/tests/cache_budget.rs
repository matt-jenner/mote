use photo_cache::{CacheBudget, EvictionPlanner, ProtectedGroups};
use photo_catalog::{Catalog, NewDerivative, NewFolderGroup, NewLibrary};
use photo_domain::{AssetId, DerivativeId, FolderGroupId, LibraryId, MediaKind, RelativePathKey};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Barrier};

const GIB: u64 = 1024 * 1024 * 1024;

#[test]
fn automatic_limit_is_ten_percent_capped_at_one_hundred_gibibytes() {
    assert_eq!(
        CacheBudget::from_total_space(500 * GIB).limit_bytes(),
        50 * GIB
    );
    assert_eq!(
        CacheBudget::from_total_space(2_000 * GIB).limit_bytes(),
        100 * GIB
    );
}

struct BudgetFixture {
    catalog: Catalog,
    root: tempfile::TempDir,
    library: LibraryId,
    asset: AssetId,
}

impl BudgetFixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = NewLibrary::configured("Photos", root.path());
        let library_id = library.id;
        catalog.add_library(&library).unwrap();
        let asset = photo_catalog::NewAsset::minimal(
            library_id,
            RelativePathKey::from_relative_path(Path::new("image.jpg")).unwrap(),
            "image.jpg",
            MediaKind::Jpeg,
            1,
        );
        let asset_id = asset.id;
        catalog.upsert_asset(&asset).unwrap();
        Self {
            catalog,
            root,
            library: library_id,
            asset: asset_id,
        }
    }

    fn group(&mut self, path: &str, viewed: i64) -> FolderGroupId {
        let id = FolderGroupId::new();
        self.catalog
            .upsert_folder_group(&NewFolderGroup {
                id,
                library_id: self.library,
                relative_path: RelativePathKey::from_relative_path(Path::new(path)).unwrap(),
                display_path: path.into(),
                last_viewed_at: Some(viewed),
            })
            .unwrap();
        id
    }

    fn row(&mut self, group: FolderGroupId, name: &str, bytes: u64, durable: bool) {
        let path = PathBuf::from(format!("{name}.jpg"));
        std::fs::write(self.root.path().join(&path), vec![0; bytes as usize]).unwrap();
        self.catalog
            .insert_derivative(&NewDerivative {
                id: DerivativeId::new(),
                asset_id: self.asset,
                folder_group_id: group,
                kind: if durable {
                    "wall_thumbnail"
                } else {
                    "screen_preview"
                }
                .into(),
                cache_key: name.into(),
                relative_cache_path: path,
                size_bytes: bytes,
                durable,
                created_at: 1,
            })
            .unwrap();
    }
}

#[test]
fn preparing_a_preview_write_evicts_one_old_group_not_individual_assets() {
    let mut fixture = BudgetFixture::new();
    let old = fixture.group("old", 1);
    let active = fixture.group("active", 2);
    let protected = fixture.group("protected", 3);
    fixture.row(old, "old-preview", 80, false);
    fixture.row(old, "old-wall", 7, true);
    fixture.row(active, "active-preview", 60, false);
    fixture.row(protected, "protected-preview", 10, false);
    let groups = ProtectedGroups::default();
    groups.protect(protected).unwrap();
    let budget = CacheBudget::from_total_space(1500);
    let result = budget
        .prepare_write(
            &mut fixture.catalog,
            fixture.root.path(),
            active,
            40,
            &groups,
        )
        .unwrap();
    assert_eq!(result.evicted_groups, vec![old]);
    assert_eq!(result.reclaimed_bytes, 80);
    assert_eq!(fixture.catalog.derivative_count(old, false).unwrap(), 0);
    assert_eq!(fixture.catalog.derivative_count(old, true).unwrap(), 1);
    assert!(!fixture.root.path().join("old-preview.jpg").exists());
    assert!(fixture.root.path().join("old-wall.jpg").exists());
    assert_eq!(fixture.catalog.derivative_count(active, false).unwrap(), 1);
    assert_eq!(
        fixture.catalog.derivative_count(protected, false).unwrap(),
        1
    );
}

#[test]
fn derivative_registration_is_idempotent_but_rejects_identity_changes() {
    let mut fixture = BudgetFixture::new();
    let group = fixture.group("one", 1);
    let value = NewDerivative {
        id: DerivativeId::new(),
        asset_id: fixture.asset,
        folder_group_id: group,
        kind: "screen_preview".into(),
        cache_key: "immutable-key".into(),
        relative_cache_path: PathBuf::from("immutable.jpg"),
        size_bytes: 10,
        durable: false,
        created_at: 1,
    };
    fixture.catalog.upsert_derivative(&value).unwrap();
    fixture.catalog.upsert_derivative(&value).unwrap();
    let mut changed = value.clone();
    changed.size_bytes = 11;
    assert!(matches!(
        fixture.catalog.upsert_derivative(&changed),
        Err(photo_catalog::CatalogError::InvalidData(_))
    ));
    changed = value.clone();
    changed.kind = "wall_thumbnail".into();
    assert!(fixture.catalog.upsert_derivative(&changed).is_err());
    changed = value.clone();
    changed.relative_cache_path = PathBuf::from("other.jpg");
    assert!(fixture.catalog.upsert_derivative(&changed).is_err());
    changed = value.clone();
    changed.durable = true;
    assert!(fixture.catalog.upsert_derivative(&changed).is_err());
    changed = value.clone();
    changed.asset_id = AssetId::for_path(
        fixture.library,
        &RelativePathKey::from_relative_path(Path::new("other.jpg")).unwrap(),
    );
    assert!(fixture.catalog.upsert_derivative(&changed).is_err());
    changed = value.clone();
    changed.folder_group_id = FolderGroupId::new();
    assert!(fixture.catalog.upsert_derivative(&changed).is_err());
    assert_eq!(fixture.catalog.derivative_count(group, false).unwrap(), 1);
}

#[test]
fn file_backed_registration_is_concurrent_and_immutable() {
    let root = tempfile::tempdir().unwrap();
    let db_path = root.path().join("catalog.sqlite3");
    let library_root = root.path().join("library");
    std::fs::create_dir_all(&library_root).unwrap();
    let library = NewLibrary::configured("Photos", &library_root);
    let asset = photo_catalog::NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("image.jpg")).unwrap(),
        "image.jpg",
        MediaKind::Jpeg,
        1,
    );
    let group = NewFolderGroup {
        id: FolderGroupId::new(),
        library_id: library.id,
        relative_path: RelativePathKey::from_relative_path(Path::new("album")).unwrap(),
        display_path: "album".into(),
        last_viewed_at: Some(1),
    };
    {
        let mut catalog = Catalog::open(&db_path).unwrap();
        catalog.add_library(&library).unwrap();
        catalog.upsert_asset(&asset).unwrap();
        catalog.upsert_folder_group(&group).unwrap();
    }
    let value = NewDerivative {
        id: DerivativeId::new(),
        asset_id: asset.id,
        folder_group_id: group.id,
        kind: "screen_preview".into(),
        cache_key: "concurrent-key".into(),
        relative_cache_path: PathBuf::from("aa/bb/concurrent.jpg"),
        size_bytes: 37,
        durable: false,
        created_at: 1,
    };
    let barrier = Arc::new(Barrier::new(2));
    let (left, right) = std::thread::scope(|scope| {
        let first = Arc::clone(&barrier);
        let second = Arc::clone(&barrier);
        let left_path = db_path.clone();
        let left_value = value.clone();
        let left = scope.spawn(move || {
            let mut catalog = Catalog::open(&left_path).unwrap();
            first.wait();
            catalog.upsert_derivative(&left_value)
        });
        let right_path = db_path.clone();
        let right_value = value.clone();
        let right = scope.spawn(move || {
            let mut catalog = Catalog::open(&right_path).unwrap();
            second.wait();
            catalog.upsert_derivative(&right_value)
        });
        (left.join().unwrap(), right.join().unwrap())
    });
    assert!(left.is_ok(), "first registration failed: {left:?}");
    assert!(right.is_ok(), "second registration failed: {right:?}");
    let mut catalog = Catalog::open(&db_path).unwrap();
    let rows = catalog.all_derivatives().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].asset_id, value.asset_id);
    assert_eq!(rows[0].folder_group_id, value.folder_group_id);
    assert_eq!(rows[0].relative_cache_path, value.relative_cache_path);
    assert_eq!(rows[0].size_bytes, value.size_bytes);
    assert_eq!(rows[0].kind, value.kind);
    assert_eq!(rows[0].durable, value.durable);

    let mut mismatch = value.clone();
    mismatch.size_bytes += 1;
    assert!(matches!(
        catalog.upsert_derivative(&mismatch),
        Err(photo_catalog::CatalogError::InvalidData(_))
    ));
    let rows_after = catalog.all_derivatives().unwrap();
    assert_eq!(rows_after.len(), 1);
    assert_eq!(rows_after[0].size_bytes, value.size_bytes);
    assert_eq!(rows_after[0].relative_cache_path, value.relative_cache_path);
}

#[test]
fn protection_is_reference_counted_and_budget_failure_keeps_cache_intact() {
    let groups = ProtectedGroups::default();
    let group = FolderGroupId::new();
    groups.protect(group).unwrap();
    let first = groups.begin_write(group).unwrap();
    let second = groups.begin_write(group).unwrap();
    drop(first);
    let root = tempfile::tempdir().unwrap();
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = NewLibrary::configured("Photos", root.path());
    let asset = photo_catalog::NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(Path::new("image.jpg")).unwrap(),
        "image.jpg",
        MediaKind::Jpeg,
        1,
    );
    catalog.add_library(&library).unwrap();
    catalog.upsert_asset(&asset).unwrap();
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: group,
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(Path::new("album")).unwrap(),
            display_path: "album".into(),
            last_viewed_at: Some(1),
        })
        .unwrap();
    let path = PathBuf::from("preview.jpg");
    std::fs::write(root.path().join(&path), [7_u8; 9]).unwrap();
    catalog
        .insert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id: asset.id,
            folder_group_id: group,
            kind: "screen_preview".into(),
            cache_key: "preview-key".into(),
            relative_cache_path: path.clone(),
            size_bytes: 9,
            durable: false,
            created_at: 1,
        })
        .unwrap();
    assert!(
        EvictionPlanner::plan(&catalog, 1, &groups)
            .unwrap()
            .groups
            .is_empty()
    );
    let result = CacheBudget::from_total_space(0).prepare_write(
        &mut catalog,
        root.path(),
        group,
        1,
        &groups,
    );
    assert!(matches!(
        result,
        Err(photo_cache::CacheError::BudgetExceeded)
    ));
    assert_eq!(catalog.all_derivatives().unwrap().len(), 1);
    assert!(root.path().join(&path).exists());
    drop(second);
    assert!(
        EvictionPlanner::plan(&catalog, 1, &groups)
            .unwrap()
            .groups
            .is_empty()
    );
    groups.unprotect(group).unwrap();
    assert_eq!(
        EvictionPlanner::plan(&catalog, 1, &groups).unwrap().groups,
        vec![group]
    );
}
