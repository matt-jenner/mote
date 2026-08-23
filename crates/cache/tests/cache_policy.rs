use std::collections::HashSet;
use std::io::{self, Write};
use std::path::PathBuf;

use photo_cache::{
    CacheError, CacheWriter, DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget,
    EvictionPlanner, ProtectedGroups,
};
use photo_catalog::{Catalog, NewDerivative, NewFolderGroup, NewLibrary};
use photo_domain::{
    AssetId, DerivativeId, FileSignature, FolderGroupId, LibraryId, MediaKind, RelativePathKey,
};

fn signature(size_bytes: u64, modified_unix_ns: i128) -> FileSignature {
    FileSignature {
        size_bytes,
        modified_unix_ns,
        sidecar_modified_unix_ns: Some(modified_unix_ns + 1),
    }
}

fn spec(signature: FileSignature, decoder: &str) -> DerivativeSpec {
    let library = LibraryId::from_uuid(uuid::Uuid::from_u128(1));
    DerivativeSpec {
        asset_id: AssetId::for_path(
            library,
            &RelativePathKey::from_relative_path(std::path::Path::new("collection/image.jpg"))
                .unwrap(),
        ),
        signature,
        orientation: 6,
        kind: DerivativeKind::ScreenPreview,
        decoder_version: decoder.to_owned(),
        colour_space: "srgb".to_owned(),
        target: DerivativeTarget::LongEdge(2048),
    }
}

#[test]
fn derivative_key_changes_with_source_or_decoder_inputs() {
    let base = spec(signature(100, 7), "decoder-1");
    let same = spec(signature(100, 7), "decoder-1");
    let changed_source = spec(signature(101, 7), "decoder-1");
    let changed_decoder = spec(signature(100, 7), "decoder-2");

    assert_eq!(DerivativeKey::compute(&base), DerivativeKey::compute(&same));
    assert_ne!(
        DerivativeKey::compute(&base),
        DerivativeKey::compute(&changed_source)
    );
    assert_ne!(
        DerivativeKey::compute(&base),
        DerivativeKey::compute(&changed_decoder)
    );

    let key = DerivativeKey::compute(&base);
    assert_eq!(key.as_str().len(), 64);
    assert!(
        key.as_str()
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    );
    assert_eq!(key.sharded_path("jpg").components().count(), 3);
}

#[test]
fn writer_commits_atomically_and_reuses_immutable_output() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();

    let first = writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"first")
        })
        .unwrap();
    let second = writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"second")
        })
        .unwrap();

    assert!(!first.reused);
    assert!(second.reused);
    assert_eq!(
        std::fs::read(temp.path().join("ab/cd/item.bin")).unwrap(),
        b"first"
    );
    assert!(partial_files(temp.path()).is_empty());
}

#[test]
fn interrupted_write_leaves_no_final_or_partial_file() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();

    let result = writer.write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
        file.write_all(b"incomplete")?;
        Err(io::Error::other("decoder stopped"))
    });

    assert!(matches!(result, Err(CacheError::Write(_))));
    assert!(!temp.path().join("ab/cd/item.bin").exists());
    assert!(partial_files(temp.path()).is_empty());
}

#[test]
fn relative_path_escape_is_rejected_before_writing() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    let outside = temp.path().parent().unwrap().join("outside-cache-test");

    let result = writer.write_atomic(PathBuf::from("../outside-cache-test"), |file| {
        file.write_all(b"unsafe")
    });

    assert!(matches!(result, Err(CacheError::PathEscape)));
    assert!(!outside.exists());
}

#[test]
fn eviction_removes_oldest_whole_group_and_keeps_durable_thumbnails() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = CacheFixture::new(temp.path());
    let old = fixture.group("old", Some(10));
    let new = fixture.group("new", Some(20));
    fixture.derivative(old, "old-preview", 40, false);
    fixture.derivative(old, "old-thumb", 5, true);
    fixture.derivative(new, "new-preview", 40, false);
    fixture.derivative(new, "new-thumb", 5, true);

    let plan = EvictionPlanner::plan(&fixture.catalog, 35, &ProtectedGroups::default()).unwrap();
    assert_eq!(plan.groups, vec![old]);
    assert_eq!(plan.reclaimable_bytes, 40);

    EvictionPlanner::execute(&mut fixture.catalog, temp.path(), &plan).unwrap();
    assert_eq!(fixture.catalog.derivative_count(old, false).unwrap(), 0);
    assert_eq!(fixture.catalog.derivative_count(old, true).unwrap(), 1);
    assert_eq!(fixture.catalog.derivative_count(new, false).unwrap(), 1);
    assert!(!temp.path().join("old-preview.bin").exists());
    assert!(temp.path().join("old-thumb.bin").exists());
}

#[test]
fn eviction_skips_protected_and_active_groups() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = CacheFixture::new(temp.path());
    let protected = fixture.group("protected", None);
    let active = fixture.group("active", Some(1));
    let eligible = fixture.group("eligible", Some(2));
    fixture.derivative(protected, "protected", 20, false);
    fixture.derivative(active, "active", 20, false);
    fixture.derivative(eligible, "eligible", 20, false);
    let groups = ProtectedGroups {
        protected: HashSet::from([protected]),
        active_writes: HashSet::from([active]),
    };

    let plan = EvictionPlanner::plan(&fixture.catalog, 10, &groups).unwrap();

    assert_eq!(plan.groups, vec![eligible]);
}

#[test]
fn startup_cleanup_removes_partials_and_missing_catalog_rows_only() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = CacheFixture::new(temp.path());
    let group = fixture.group("collection", Some(1));
    fixture.derivative(group, "present", 7, false);
    fixture.derivative_row_only(group, "missing", 9, false);
    std::fs::write(temp.path().join("orphan.partial-123"), b"partial").unwrap();

    let writer = CacheWriter::new(temp.path()).unwrap();
    let repaired = writer.reconcile_catalog(&mut fixture.catalog).unwrap();

    assert_eq!(repaired.partial_files_removed, 1);
    assert_eq!(repaired.missing_rows_removed, 1);
    assert_eq!(fixture.catalog.derivative_count(group, false).unwrap(), 1);
    assert!(temp.path().join("present.bin").exists());
}

fn partial_files(root: &std::path::Path) -> Vec<PathBuf> {
    walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file())
        .filter(|entry| entry.file_name().to_string_lossy().contains(".partial-"))
        .map(|entry| entry.into_path())
        .collect()
}

struct CacheFixture {
    catalog: Catalog,
    library_id: LibraryId,
    asset_id: AssetId,
    cache_root: PathBuf,
}

impl CacheFixture {
    fn new(cache_root: &std::path::Path) -> Self {
        let mut catalog = Catalog::open_in_memory().unwrap();
        let library = NewLibrary::configured("Photos", cache_root);
        let library_id = library.id;
        catalog.add_library(&library).unwrap();
        let asset = photo_catalog::NewAsset::minimal(
            library_id,
            RelativePathKey::from_relative_path(std::path::Path::new("collection/image.jpg"))
                .unwrap(),
            "collection/image.jpg",
            MediaKind::Jpeg,
            100,
        );
        let asset_id = asset.id;
        catalog.upsert_asset(&asset).unwrap();
        Self {
            catalog,
            library_id,
            asset_id,
            cache_root: cache_root.to_owned(),
        }
    }

    fn group(&mut self, name: &str, last_viewed_at: Option<i64>) -> FolderGroupId {
        let id = FolderGroupId::new();
        self.catalog
            .upsert_folder_group(&NewFolderGroup {
                id,
                library_id: self.library_id,
                relative_path: RelativePathKey::from_relative_path(std::path::Path::new(name))
                    .unwrap(),
                display_path: name.to_owned(),
                last_viewed_at,
            })
            .unwrap();
        id
    }

    fn derivative(&mut self, group: FolderGroupId, name: &str, bytes: u64, durable: bool) {
        std::fs::write(
            self.cache_root.join(format!("{name}.bin")),
            vec![0; bytes as usize],
        )
        .unwrap();
        self.derivative_row_only(group, name, bytes, durable);
    }

    fn derivative_row_only(&mut self, group: FolderGroupId, name: &str, bytes: u64, durable: bool) {
        self.catalog
            .insert_derivative(&NewDerivative {
                id: DerivativeId::new(),
                asset_id: self.asset_id,
                folder_group_id: group,
                kind: if durable {
                    "wall_thumbnail".to_owned()
                } else {
                    "screen_preview".to_owned()
                },
                cache_key: name.to_owned(),
                relative_cache_path: PathBuf::from(format!("{name}.bin")),
                size_bytes: bytes,
                durable,
                created_at: 1,
            })
            .unwrap();
    }
}
