use std::fs;
use std::path::{Path, PathBuf};

use photo_app_service::{AppConfig, AppService, AppServiceError, CopyItemStatus};
use photo_catalog::{AssetShapeUpdate, Catalog, CatalogIndexRecord, NewAsset, ShapeStatus};
use photo_domain::{FolderGroupId, MediaKind, NativePathKey, RelativePathKey};

struct Fixture {
    temp: tempfile::TempDir,
    config: AppConfig,
    service: AppService,
    roots: Vec<PathBuf>,
    groups: Vec<String>,
    ids: Vec<String>,
    sources: Vec<PathBuf>,
    destination: PathBuf,
}

impl Fixture {
    fn new(names: &[&str]) -> Self {
        let temp = tempfile::tempdir().unwrap();
        let config = AppConfig::new(temp.path().join("data"), temp.path().join("cache"));
        let service = AppService::open(config.clone()).unwrap();
        let destination = temp.path().join("exports");
        fs::create_dir(&destination).unwrap();
        let mut fixture = Self {
            temp,
            config,
            service,
            destination,
            roots: vec![],
            groups: vec![],
            ids: vec![],
            sources: vec![],
        };
        for (index, name) in names.iter().enumerate() {
            let root = fixture.temp.path().join(format!("library-{index}"));
            fs::create_dir_all(root.join("child")).unwrap();
            let relative = Path::new("child").join(name);
            let source = root.join(&relative);
            let mut bytes = format!("original bytes {index}").into_bytes();
            bytes.extend_from_slice(&[0, 255, 128]);
            fs::write(&source, bytes).unwrap();
            let bootstrap = fixture.service.open_recent(&root).unwrap();
            let saved = bootstrap
                .saved_folders
                .entries
                .iter()
                .find(|entry| entry.name == format!("library-{index}"))
                .unwrap();
            let group = FolderGroupId::from_uuid(uuid::Uuid::parse_str(&saved.folder_id).unwrap());
            let mut catalog = Catalog::open(&fixture.config.catalog_path()).unwrap();
            let library = catalog.folder_group(group).unwrap().unwrap().library_id;
            let mut asset = NewAsset::minimal(
                library,
                RelativePathKey::from_relative_path(&relative).unwrap(),
                "untrusted/display/name.jpg",
                MediaKind::Jpeg,
                1,
            );
            asset.folder_group_id = Some(group);
            catalog.upsert_asset(&asset).unwrap();
            catalog
                .apply_index_batch(&[CatalogIndexRecord::Shaped(AssetShapeUpdate {
                    asset_id: asset.id,
                    width: 3,
                    height: 2,
                    orientation: Some(1),
                    representative_rgb: None,
                    shape_status: ShapeStatus::Ready,
                })])
                .unwrap();
            let id = asset.id.as_uuid().to_string();
            fixture
                .service
                .add_photo_pick(&id, &saved.folder_id)
                .unwrap();
            fixture.roots.push(root);
            fixture.groups.push(saved.folder_id.clone());
            fixture.ids.push(id);
            fixture.sources.push(source);
        }
        fixture
    }

    fn remembered(&self) -> Option<NativePathKey> {
        Catalog::open(&self.config.catalog_path())
            .unwrap()
            .last_copy_destination()
            .unwrap()
    }
}

fn names(directory: &Path) -> Vec<String> {
    let mut names = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    names.sort();
    names
}

// Catches re-encoding, source writes, nested output, and use of display paths.
#[test]
fn copies_flat_original_bytes_without_changing_source_hash_or_metadata() {
    let fixture = Fixture::new(&["IMG_2048.jpg"]);
    let source = &fixture.sources[0];
    let mut permissions = fs::metadata(source).unwrap().permissions();
    permissions.set_readonly(true);
    fs::set_permissions(source, permissions).unwrap();
    let before = fs::metadata(source).unwrap();
    let bytes = fs::read(source).unwrap();
    let digest = blake3::hash(&bytes);
    let batch = fixture.service.prepare_original_copy(&fixture.ids).unwrap();
    assert_eq!(fixture.remembered(), None);
    let result = fixture
        .service
        .copy_originals(batch, &fixture.destination)
        .unwrap();
    assert_eq!((result.copied_count, result.failed_count), (1, 0));
    assert_eq!(result.items[0].status, CopyItemStatus::Copied);
    assert_eq!(
        result.items[0].destination_name.as_deref(),
        Some("IMG_2048.jpg")
    );
    assert_eq!(names(&fixture.destination), ["IMG_2048.jpg"]);
    assert_eq!(
        fs::read(fixture.destination.join("IMG_2048.jpg")).unwrap(),
        bytes
    );
    assert_eq!(blake3::hash(&fs::read(source).unwrap()), digest);
    let after = fs::metadata(source).unwrap();
    assert_eq!(after.len(), before.len());
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(after.permissions(), before.permissions());
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            (after.ino(), after.mode(), after.ctime(), after.ctime_nsec()),
            (
                before.ino(),
                before.mode(),
                before.ctime(),
                before.ctime_nsec()
            )
        );
    }
    assert_eq!(names(&fixture.roots[0]), ["child"]);
    assert_eq!(names(&fixture.roots[0].join("child")), ["IMG_2048.jpg"]);
    assert_eq!(
        fixture.remembered(),
        Some(NativePathKey::from_path(
            &fs::canonicalize(&fixture.destination).unwrap()
        ))
    );
    let wire = serde_json::to_value(&result).unwrap();
    assert_eq!(wire["items"][0]["assetId"], fixture.ids[0]);
    assert_eq!(wire["items"][0]["status"], "copied");
    assert!(
        !wire
            .to_string()
            .contains(fixture.temp.path().to_str().unwrap())
    );
}

// Catches overwrite and independent item reservations that choose the same name.
#[test]
fn reserves_case_insensitive_names_from_existing_entries_and_the_whole_batch() {
    let fixture = Fixture::new(&["IMG_2048.jpg", "img_2048.JPG", "IMG_2048.jpg"]);
    fs::write(fixture.destination.join("img_2048.jpg"), b"keep me").unwrap();
    fs::create_dir(fixture.destination.join("IMG_2048 (2).JPG")).unwrap();
    let result = fixture
        .service
        .copy_originals(
            fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
            &fixture.destination,
        )
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|item| item.destination_name.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["IMG_2048 (3).jpg", "img_2048 (4).JPG", "IMG_2048 (5).jpg"]
    );
    assert_eq!(
        fs::read(fixture.destination.join("img_2048.jpg")).unwrap(),
        b"keep me"
    );
    assert!(fixture.destination.join("IMG_2048 (2).JPG").is_dir());
}

// Catches suffixes incorrectly appended after the extension or lost extensions.
#[test]
fn uses_numeric_suffixes_for_duplicate_basenames_with_and_without_extensions() {
    let fixture = Fixture::new(&["IMG_2048.jpg", "IMG_2048.jpg", "README", "README"]);
    let result = fixture
        .service
        .copy_originals(
            fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
            &fixture.destination,
        )
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|item| item.destination_name.as_deref().unwrap())
            .collect::<Vec<_>>(),
        ["IMG_2048.jpg", "IMG_2048 (2).jpg", "README", "README (2)"]
    );
}

// Catches lazy pick lookup or mutation of the prepared sequence.
#[test]
fn prepared_batch_keeps_authorized_order_when_picks_change_before_and_during_copy() {
    let fixture = Fixture::new(&["first.jpg", "second.jpg", "later.jpg"]);
    fixture.service.remove_photo_pick(&fixture.ids[2]).unwrap();
    let batch = fixture
        .service
        .prepare_original_copy(&[fixture.ids[1].clone(), fixture.ids[0].clone()])
        .unwrap();
    fixture.service.clear_photo_picks().unwrap();
    fixture
        .service
        .add_photo_pick(&fixture.ids[2], &fixture.groups[2])
        .unwrap();
    let result = fixture
        .service
        .copy_originals_with_progress(batch, &fixture.destination, |_| {
            fixture.service.clear_photo_picks().unwrap();
        })
        .unwrap();
    assert_eq!(
        result
            .items
            .iter()
            .map(|item| &item.asset_id)
            .collect::<Vec<_>>(),
        [&fixture.ids[1], &fixture.ids[0]]
    );
    assert_eq!(names(&fixture.destination), ["first.jpg", "second.jpg"]);
    assert!(fixture.service.list_photo_picks().unwrap().items.is_empty());
}

// Catches arbitrary IDs authorizing a copy and duplicate IDs multiplying copies.
#[test]
fn preparation_rejects_non_picks_malformed_ids_and_empty_or_oversized_requests() {
    let fixture = Fixture::new(&["first.jpg", "second.jpg"]);
    fixture.service.remove_photo_pick(&fixture.ids[1]).unwrap();
    assert!(matches!(
        fixture.service.prepare_original_copy(&fixture.ids),
        Err(AppServiceError::ForeignAsset)
    ));
    assert!(matches!(
        fixture.service.prepare_original_copy(&["bad".into()]),
        Err(AppServiceError::InvalidAssetId)
    ));
    assert!(fixture.service.prepare_original_copy(&[]).is_err());
    assert!(
        fixture
            .service
            .prepare_original_copy(&vec![fixture.ids[0].clone(); 251])
            .is_err()
    );
    let batch = fixture
        .service
        .prepare_original_copy(&[fixture.ids[0].clone(), fixture.ids[0].clone()])
        .unwrap();
    let result = fixture
        .service
        .copy_originals(batch, &fixture.destination)
        .unwrap();
    assert_eq!(result.items.len(), 1);
}

// Catches aborting a mixed batch and retrying already successful items.
#[test]
fn missing_source_is_an_item_failure_and_only_failed_ids_need_retry() {
    let fixture = Fixture::new(&["missing.jpg", "good.jpg"]);
    let missing_bytes = fs::read(&fixture.sources[0]).unwrap();
    fs::remove_file(&fixture.sources[0]).unwrap();
    let first = fixture
        .service
        .copy_originals(
            fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
            &fixture.destination,
        )
        .unwrap();
    assert_eq!((first.copied_count, first.failed_count), (1, 1));
    assert_eq!(first.items[0].status, CopyItemStatus::Failed);
    assert_eq!(
        first.items[0].error_code.as_deref(),
        Some("source_unavailable")
    );
    assert_eq!(first.items[0].destination_name, None);
    let failed = first
        .items
        .iter()
        .filter(|item| item.status == CopyItemStatus::Failed)
        .map(|item| item.asset_id.clone())
        .collect::<Vec<_>>();
    fs::write(&fixture.sources[0], missing_bytes).unwrap();
    let retry = fixture
        .service
        .copy_originals(
            fixture.service.prepare_original_copy(&failed).unwrap(),
            &fixture.destination,
        )
        .unwrap();
    assert_eq!((retry.copied_count, retry.failed_count), (1, 0));
    assert_eq!(names(&fixture.destination), ["good.jpg", "missing.jpg"]);
    assert!(
        !serde_json::to_string(&first)
            .unwrap()
            .contains(fixture.temp.path().to_str().unwrap())
    );
}

// Catches treating preparation-time availability as a guarantee at execution.
#[test]
fn source_disappearing_after_preparation_does_not_leave_an_empty_destination_file() {
    let fixture = Fixture::new(&["gone.jpg"]);
    let batch = fixture.service.prepare_original_copy(&fixture.ids).unwrap();
    fs::remove_file(&fixture.sources[0]).unwrap();
    let result = fixture
        .service
        .copy_originals(batch, &fixture.destination)
        .unwrap();
    assert_eq!((result.copied_count, result.failed_count), (0, 1));
    assert!(names(&fixture.destination).is_empty());
    assert_eq!(fixture.remembered(), None);
}

// Catches opening unreadable originals as writeable or claiming an empty copy succeeded.
#[cfg(unix)]
#[test]
fn unreadable_source_fails_without_changing_permissions_or_preference() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new(&["unreadable.jpg"]);
    fs::set_permissions(&fixture.sources[0], fs::Permissions::from_mode(0o000)).unwrap();
    assert!(
        fs::File::open(&fixture.sources[0]).is_err(),
        "this permission test requires a non-root runner"
    );
    let result = fixture
        .service
        .copy_originals(
            fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
            &fixture.destination,
        )
        .unwrap();
    assert_eq!((result.copied_count, result.failed_count), (0, 1));
    assert_eq!(
        result.items[0].error_code.as_deref(),
        Some("source_unavailable")
    );
    assert_eq!(
        fs::metadata(&fixture.sources[0])
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0
    );
    assert!(names(&fixture.destination).is_empty());
    assert_eq!(fixture.remembered(), None);
}

// Catches retry loops recreating lost directories or starting remaining items.
#[test]
fn destination_loss_mid_batch_stops_and_keeps_completed_copy() {
    let fixture = Fixture::new(&["first.jpg", "second.jpg", "third.jpg"]);
    let moved = fixture.temp.path().join("unmounted");
    let mut reports = 0;
    let result = fixture.service.copy_originals_with_progress(
        fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
        &fixture.destination,
        |_| {
            reports += 1;
            if reports == 1 {
                fs::rename(&fixture.destination, &moved).unwrap();
            }
        },
    );
    assert!(matches!(
        result,
        Err(AppServiceError::CopyDestinationMissing)
    ));
    assert_eq!(reports, 1);
    assert_eq!(names(&moved), ["first.jpg"]);
    assert!(!fixture.destination.exists());
    assert!(fixture.remembered().is_none());
}

// Catches stale destination scans overwriting files that appear during an operation.
#[test]
fn competing_destination_entry_is_preserved_and_gets_a_new_suffix() {
    let fixture = Fixture::new(&["first.jpg", "second.jpg"]);
    let result = fixture
        .service
        .copy_originals_with_progress(
            fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
            &fixture.destination,
            |item| {
                if item.asset_id == fixture.ids[0] {
                    fs::write(fixture.destination.join("SECOND.JPG"), b"other process").unwrap();
                }
            },
        )
        .unwrap();
    assert_eq!(
        result.items[1].destination_name.as_deref(),
        Some("second (2).jpg")
    );
    assert_eq!(
        fs::read(fixture.destination.join("SECOND.JPG")).unwrap(),
        b"other process"
    );
}

// Catches checking only the picked library, lexical prefix checks, and preference writes on rejection.
#[test]
fn destination_equal_to_or_below_any_library_is_rejected() {
    let fixture = Fixture::new(&["first.jpg", "second.jpg"]);
    fixture.service.remove_photo_pick(&fixture.ids[1]).unwrap();
    for root in &fixture.roots {
        for destination in [root.clone(), root.join("child")] {
            let error = fixture
                .service
                .copy_originals(
                    fixture
                        .service
                        .prepare_original_copy(&fixture.ids[..1])
                        .unwrap(),
                    &destination,
                )
                .unwrap_err();
            assert!(matches!(error, AppServiceError::CopyDestinationIsSource));
            assert!(!error.to_string().contains(root.to_str().unwrap()));
        }
    }
    assert_eq!(fixture.remembered(), None);
}

// Catches use of a stale root list when another source is configured after preparation.
#[test]
fn destination_added_as_library_after_preparation_is_rejected() {
    let fixture = Fixture::new(&["first.jpg"]);
    let batch = fixture.service.prepare_original_copy(&fixture.ids).unwrap();
    fixture.service.open_recent(&fixture.destination).unwrap();
    assert!(matches!(
        fixture.service.copy_originals(batch, &fixture.destination),
        Err(AppServiceError::CopyDestinationIsSource)
    ));
    assert!(names(&fixture.destination).is_empty());
}

// Catches implicit destination creation and unsafe acceptance of a file path.
#[test]
fn missing_or_non_directory_destination_is_a_bounded_error() {
    let fixture = Fixture::new(&["first.jpg"]);
    let file = fixture.temp.path().join("ordinary-file");
    fs::write(&file, b"preserve this file").unwrap();
    for destination in [fixture.temp.path().join("absent"), file.clone()] {
        let error = fixture
            .service
            .copy_originals(
                fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
                &destination,
            )
            .unwrap_err();
        assert!(matches!(error, AppServiceError::CopyDestinationUnavailable));
        assert!(
            !error
                .to_string()
                .contains(fixture.temp.path().to_str().unwrap())
        );
    }
    assert!(!fixture.temp.path().join("absent").exists());
    assert_eq!(fs::read(file).unwrap(), b"preserve this file");
    assert_eq!(fixture.remembered(), None);
}

// Catches symlink aliases bypassing destination containment or source authorization.
#[cfg(unix)]
#[test]
fn symlinks_cannot_hide_source_destinations_or_escape_authorized_sources() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(&["first.jpg", "second.jpg"]);
    let alias = fixture.temp.path().join("source-alias");
    symlink(&fixture.roots[1], &alias).unwrap();
    assert!(matches!(
        fixture.service.copy_originals(
            fixture
                .service
                .prepare_original_copy(&fixture.ids[..1])
                .unwrap(),
            &alias,
        ),
        Err(AppServiceError::CopyDestinationIsSource)
    ));
    let outside = fixture.temp.path().join("private.jpg");
    fs::write(&outside, b"must not export").unwrap();
    fs::remove_file(&fixture.sources[0]).unwrap();
    symlink(&outside, &fixture.sources[0]).unwrap();
    let result = fixture
        .service
        .copy_originals(
            fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
            &fixture.destination,
        )
        .unwrap();
    assert_eq!((result.copied_count, result.failed_count), (1, 1));
    assert_eq!(
        result.items[0].error_code.as_deref(),
        Some("source_unavailable")
    );
    assert_eq!(names(&fixture.destination), ["second.jpg"]);
    assert_eq!(fs::read(&outside).unwrap(), b"must not export");
}

// Catches validating the destination only once, then following a substituted symlink into a source.
#[cfg(unix)]
#[test]
fn destination_swapped_to_source_between_items_is_rejected_without_source_writes() {
    use std::os::unix::fs::symlink;
    let fixture = Fixture::new(&["first.jpg", "second.jpg"]);
    let moved = fixture.temp.path().join("old-export");
    let result = fixture
        .service
        .copy_originals_with_progress(
            fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
            &fixture.destination,
            |item| {
                if item.asset_id == fixture.ids[0] {
                    fs::rename(&fixture.destination, &moved).unwrap();
                    symlink(&fixture.roots[1], &fixture.destination).unwrap();
                }
            },
        )
        .unwrap();
    assert_eq!((result.copied_count, result.failed_count), (1, 1));
    assert_eq!(names(&fixture.roots[1]), ["child"]);
    assert_eq!(names(&moved), ["first.jpg"]);
}

// Catches remembered-destination updates caused by cancellation or zero-success failures.
#[test]
fn dropped_batch_and_failed_operation_preserve_previous_destination() {
    let fixture = Fixture::new(&["first.jpg"]);
    let old = NativePathKey::from_path(&fixture.temp.path().join("previous-export"));
    Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .set_last_copy_destination(&old)
        .unwrap();
    drop(fixture.service.prepare_original_copy(&fixture.ids).unwrap());
    assert_eq!(fixture.remembered(), Some(old.clone()));
    fs::remove_file(&fixture.sources[0]).unwrap();
    let result = fixture
        .service
        .copy_originals(
            fixture.service.prepare_original_copy(&fixture.ids).unwrap(),
            &fixture.destination,
        )
        .unwrap();
    assert_eq!(result.copied_count, 0);
    assert_eq!(fixture.remembered(), Some(old));
}

// Catches real write failures leaving partial output or changing the source.
// Limit only a child process, so parallel service tests keep their normal limits.
#[cfg(unix)]
#[test]
fn filesystem_write_failure_removes_unreadable_partial_output_and_preserves_source() {
    const CHILD_ROOT: &str = "PHOTO_COPY_WRITE_FAILURE_CHILD_ROOT";
    const CHILD_ID: &str = "PHOTO_COPY_WRITE_FAILURE_CHILD_ID";
    if let Some(root) = std::env::var_os(CHILD_ROOT) {
        let root = PathBuf::from(root);
        let service =
            AppService::open(AppConfig::new(root.join("data"), root.join("cache"))).unwrap();
        let batch = service
            .prepare_original_copy(&[std::env::var(CHILD_ID).unwrap()])
            .unwrap();
        // Constrain only copy and its permission probe, not catalog startup.
        // SAFETY: this branch runs in a dedicated child process with one test.
        unsafe {
            assert_ne!(libc::signal(libc::SIGXFSZ, libc::SIG_IGN), libc::SIG_ERR);
            let limit = libc::rlimit {
                rlim_cur: 16 * 1024,
                rlim_max: 16 * 1024,
            };
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
            libc::umask(0o444);
        }
        let probe = root.join("exports/write-only-probe");
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&probe)
            .unwrap();
        assert!(
            fs::File::open(&probe).is_err(),
            "restrictive umask must deny data reads"
        );
        fs::remove_file(probe).unwrap();
        let result = service
            .copy_originals(batch, &root.join("exports"))
            .unwrap();
        assert_eq!((result.copied_count, result.failed_count), (0, 1));
        assert_eq!(result.items[0].error_code.as_deref(), Some("copy_failed"));
        assert_eq!(names(&root.join("exports")), ["existing.jpg"]);
        return;
    }
    let fixture = Fixture::new(&["large.jpg"]);
    let bytes = vec![0xa5; 128 * 1024];
    fs::write(&fixture.sources[0], &bytes).unwrap();
    let before = fs::metadata(&fixture.sources[0]).unwrap();
    fs::write(fixture.destination.join("existing.jpg"), b"keep existing").unwrap();
    // Force startup to update the catalog regardless of the wall-clock second.
    // The copy failure injection must not also constrain that setup write.
    let group = FolderGroupId::from_uuid(uuid::Uuid::parse_str(&fixture.groups[0]).unwrap());
    Catalog::open(&fixture.config.catalog_path())
        .unwrap()
        .touch_folder_group(group, 0)
        .unwrap();
    let child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "filesystem_write_failure_removes_unreadable_partial_output_and_preserves_source",
            "--nocapture",
        ])
        .env(CHILD_ROOT, fixture.temp.path())
        .env(CHILD_ID, &fixture.ids[0])
        .output()
        .unwrap();
    assert!(
        child.status.success(),
        "child failed: {}\n{}",
        String::from_utf8_lossy(&child.stdout),
        String::from_utf8_lossy(&child.stderr)
    );
    assert_eq!(names(&fixture.destination), ["existing.jpg"]);
    assert_eq!(
        fs::read(fixture.destination.join("existing.jpg")).unwrap(),
        b"keep existing"
    );
    assert_eq!(
        blake3::hash(&fs::read(&fixture.sources[0]).unwrap()),
        blake3::hash(&bytes)
    );
    assert_eq!(
        fs::metadata(&fixture.sources[0])
            .unwrap()
            .modified()
            .unwrap(),
        before.modified().unwrap()
    );
    assert_eq!(fixture.remembered(), None);
}
