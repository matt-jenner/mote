use photo_cache::{CacheBudget, ProtectedGroups};
use photo_catalog::{Catalog, NewDerivative, NewFolderGroup, NewLibrary};
use photo_domain::{AssetId, DerivativeId, FolderGroupId, LibraryId, MediaKind, RelativePathKey};
use std::path::{Path, PathBuf};

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
    let budget = CacheBudget::from_total_space(1000);
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
