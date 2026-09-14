use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::{Arc, Barrier, mpsc};
use std::thread;
use std::time::Duration;

use photo_cache::{
    CacheError, CacheWriter, DerivativeKey, DerivativeKind, DerivativeSpec, DerivativeTarget,
    EvictionPlanner, ProtectedGroups,
};
use photo_catalog::{Catalog, CatalogError, NewDerivative, NewFolderGroup, NewLibrary};
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
        media_kind: MediaKind::Jpeg,
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
fn existing_jpeg_derivative_key_golden_vector() {
    let key = DerivativeKey::compute(&spec(signature(100, 7), "image-0.25-v1"));
    assert_eq!(
        key.as_str(),
        "4ab8932a6927d5221d38396a9173f8f6a195d33dd2b16a9c0217bc0a4455487f"
    );
}

#[test]
fn heif_derivative_keys_use_a_separate_media_domain() {
    let jpeg = spec(signature(100, 7), "image-0.25-v1");
    for media_kind in [MediaKind::Png, MediaKind::Tiff, MediaKind::Webp] {
        assert_eq!(
            DerivativeKey::compute(&jpeg),
            DerivativeKey::compute(&DerivativeSpec {
                media_kind,
                ..jpeg.clone()
            }),
            "legacy media kinds must not add key bytes"
        );
    }
    let heif = DerivativeSpec {
        media_kind: MediaKind::Heif,
        decoder_version: "libheif-1.23.4-libde265-1.1.1-sdr-v1".to_owned(),
        ..jpeg.clone()
    };
    let heif_with_legacy_fingerprint = DerivativeSpec {
        decoder_version: jpeg.decoder_version.clone(),
        ..heif.clone()
    };

    assert_ne!(DerivativeKey::compute(&jpeg), DerivativeKey::compute(&heif));
    assert_ne!(
        DerivativeKey::compute(&jpeg),
        DerivativeKey::compute(&heif_with_legacy_fingerprint),
        "HEIF key isolation must not depend only on the decoder fingerprint"
    );
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
fn open_checked_opens_only_a_managed_regular_file() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"managed")
        })
        .unwrap();

    let mut file = writer
        .open_checked(std::path::Path::new("ab/cd/item.bin"))
        .unwrap();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes).unwrap();
    assert_eq!(bytes, b"managed");
    assert!(matches!(
        writer.open_checked(std::path::Path::new("../item.bin")),
        Err(CacheError::PathEscape)
    ));
}

#[cfg(unix)]
#[test]
fn open_checked_rejects_a_fifo_without_blocking_or_opening_it_as_media() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    let fifo = temp.path().join("fifo");
    use std::os::unix::ffi::OsStrExt;
    let fifo_name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_name.as_ptr(), 0o600) }, 0);

    assert!(matches!(
        writer.open_checked(std::path::Path::new("fifo")),
        Err(CacheError::PathEscape)
    ));
}

#[cfg(unix)]
#[test]
fn remove_checked_rejects_a_symlinked_ancestor_without_touching_outside() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(&temp.path().join("cache")).unwrap();
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("item.bin"), b"outside").unwrap();
    std::os::unix::fs::symlink(&outside, temp.path().join("cache/escape")).unwrap();

    assert!(matches!(
        writer.remove_checked(std::path::Path::new("escape/item.bin")),
        Err(CacheError::PathEscape) | Err(CacheError::Io(_))
    ));
    assert_eq!(std::fs::read(outside.join("item.bin")).unwrap(), b"outside");
}

#[cfg(unix)]
#[test]
fn open_checked_survives_an_ancestor_swap_at_the_final_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let writer = CacheWriter::new(&cache).unwrap();
    writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"managed")
        })
        .unwrap();
    std::fs::write(outside.join("item.bin"), b"outside").unwrap();

    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let hook_entered = entered.clone();
    let hook_release = release.clone();
    writer.install_path_race_test_hook(Arc::new(move || {
        hook_entered.wait();
        hook_release.wait();
    }));
    let reader = writer.clone();
    let opened = thread::spawn(move || reader.open_checked(std::path::Path::new("ab/cd/item.bin")));
    entered.wait();
    std::fs::rename(cache.join("ab"), cache.join("ab-moved")).unwrap();
    std::os::unix::fs::symlink(&outside, cache.join("ab")).unwrap();
    release.wait();
    let mut file = opened.join().unwrap().unwrap();
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut file, &mut bytes).unwrap();
    assert_eq!(bytes, b"managed");
    assert_eq!(std::fs::read(outside.join("item.bin")).unwrap(), b"outside");
}

#[cfg(unix)]
#[test]
fn remove_checked_survives_an_ancestor_swap_at_the_final_boundary() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let writer = CacheWriter::new(&cache).unwrap();
    writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"managed")
        })
        .unwrap();
    std::fs::write(outside.join("item.bin"), b"outside").unwrap();

    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let hook_entered = entered.clone();
    let hook_release = release.clone();
    writer.install_path_race_test_hook(Arc::new(move || {
        hook_entered.wait();
        hook_release.wait();
    }));
    let remover = writer.clone();
    let removed =
        thread::spawn(move || remover.remove_checked(std::path::Path::new("ab/cd/item.bin")));
    entered.wait();
    std::fs::rename(cache.join("ab"), cache.join("ab-moved")).unwrap();
    std::os::unix::fs::symlink(&outside, cache.join("ab")).unwrap();
    release.wait();
    removed.join().unwrap().unwrap();
    assert!(!cache.join("ab-moved/cd/item.bin").exists());
    assert_eq!(std::fs::read(outside.join("item.bin")).unwrap(), b"outside");
}

#[cfg(unix)]
#[test]
fn open_checked_rejects_a_final_component_swap_to_a_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let writer = CacheWriter::new(&cache).unwrap();
    writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"managed")
        })
        .unwrap();
    std::fs::write(outside.join("item.bin"), b"outside").unwrap();

    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let hook_entered = entered.clone();
    let hook_release = release.clone();
    writer.install_path_race_test_hook(Arc::new(move || {
        hook_entered.wait();
        hook_release.wait();
    }));
    let reader = writer.clone();
    let opened = thread::spawn(move || reader.open_checked(std::path::Path::new("ab/cd/item.bin")));
    entered.wait();
    std::fs::remove_file(cache.join("ab/cd/item.bin")).unwrap();
    std::os::unix::fs::symlink(outside.join("item.bin"), cache.join("ab/cd/item.bin")).unwrap();
    release.wait();
    assert!(matches!(
        opened.join().unwrap(),
        Err(CacheError::Io(_)) | Err(CacheError::PathEscape)
    ));
    assert_eq!(std::fs::read(outside.join("item.bin")).unwrap(), b"outside");
}

#[cfg(unix)]
#[test]
fn remove_checked_rejects_a_final_component_swap_to_a_symlink() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let writer = CacheWriter::new(&cache).unwrap();
    writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"managed")
        })
        .unwrap();
    std::fs::write(outside.join("item.bin"), b"outside").unwrap();

    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let hook_entered = entered.clone();
    let hook_release = release.clone();
    writer.install_path_race_test_hook(Arc::new(move || {
        hook_entered.wait();
        hook_release.wait();
    }));
    let remover = writer.clone();
    let removed =
        thread::spawn(move || remover.remove_checked(std::path::Path::new("ab/cd/item.bin")));
    entered.wait();
    std::fs::remove_file(cache.join("ab/cd/item.bin")).unwrap();
    std::os::unix::fs::symlink(outside.join("item.bin"), cache.join("ab/cd/item.bin")).unwrap();
    release.wait();
    assert!(matches!(
        removed.join().unwrap(),
        Err(CacheError::PathEscape)
    ));
    assert_eq!(std::fs::read(outside.join("item.bin")).unwrap(), b"outside");
}

#[cfg(windows)]
#[test]
fn windows_open_checked_rejects_a_reparse_point_final_component() {
    use std::os::windows::fs::symlink_file;

    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    let outside = temp.path().join("outside.bin");
    std::fs::write(&outside, b"outside").unwrap();
    symlink_file(&outside, temp.path().join("item.bin")).unwrap();
    assert!(matches!(
        writer.open_checked(std::path::Path::new("item.bin")),
        Err(CacheError::PathEscape) | Err(CacheError::Io(_))
    ));
}

#[cfg(windows)]
#[test]
fn windows_replacement_creates_a_missing_destination() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();

    let replaced = writer
        .replace_atomic(PathBuf::from("ab/cd/missing.jpg"), |file| {
            file.write_all(b"first generation")
        })
        .unwrap();

    assert!(!replaced.reused);
    assert_eq!(
        std::fs::read(temp.path().join("ab/cd/missing.jpg")).unwrap(),
        b"first generation"
    );
    assert!(partial_files(temp.path()).is_empty());
}

#[cfg(windows)]
#[test]
fn windows_native_replacement_failure_preserves_old_and_outside_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    let outside = temp.path().join("outside-source-sentinel.jpg");
    std::fs::write(&outside, b"outside source sentinel").unwrap();
    writer
        .write_atomic(PathBuf::from("shared.jpg"), |file| file.write_all(b"old"))
        .unwrap();
    writer.fail_next_replace_for_test();
    assert!(
        writer
            .replace_atomic(PathBuf::from("shared.jpg"), |file| file.write_all(b"new"))
            .is_err()
    );
    assert_eq!(
        std::fs::read(temp.path().join("shared.jpg")).unwrap(),
        b"old"
    );
    assert_eq!(std::fs::read(outside).unwrap(), b"outside source sentinel");
    assert!(partial_files(temp.path()).is_empty());
}

#[cfg(windows)]
#[test]
fn windows_replacement_boundary_pins_ancestors_against_a_reparse_swap() {
    use std::os::windows::fs::symlink_dir;
    use std::sync::atomic::{AtomicBool, Ordering};

    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let outside = temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("item.bin"), b"outside sentinel").unwrap();
    let writer = CacheWriter::new(&cache).unwrap();
    writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"old")
        })
        .unwrap();

    let swap_blocked = Arc::new(AtomicBool::new(false));
    let hook_swap_blocked = Arc::clone(&swap_blocked);
    let ancestor = cache.join("ab");
    let backup = cache.join("ab-old");
    let hook_outside = outside.clone();
    writer.install_replacement_test_hook(Arc::new(move || {
        match std::fs::rename(&ancestor, &backup) {
            Ok(()) => {
                symlink_dir(&hook_outside, &ancestor).unwrap();
            }
            Err(_) => hook_swap_blocked.store(true, Ordering::Release),
        }
    }));

    writer
        .replace_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"new")
        })
        .unwrap();

    assert!(swap_blocked.load(Ordering::Acquire));
    assert_eq!(std::fs::read(cache.join("ab/cd/item.bin")).unwrap(), b"new");
    assert_eq!(
        std::fs::read(outside.join("item.bin")).unwrap(),
        b"outside sentinel"
    );
}

#[cfg(windows)]
#[test]
fn windows_handle_relative_replacement_does_not_follow_a_final_reparse_swap() {
    use std::os::windows::fs::symlink_file;

    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    let destination = temp.path().join("item.bin");
    let outside = temp.path().join("outside.bin");
    let backup = temp.path().join("item.old");
    std::fs::write(&outside, b"outside sentinel").unwrap();
    writer
        .write_atomic(PathBuf::from("item.bin"), |file| file.write_all(b"old"))
        .unwrap();

    let hook_destination = destination.clone();
    let hook_backup = backup.clone();
    let hook_outside = outside.clone();
    writer.install_replacement_test_hook(Arc::new(move || {
        std::fs::rename(&hook_destination, &hook_backup).unwrap();
        symlink_file(&hook_outside, &hook_destination).unwrap();
    }));
    writer
        .replace_atomic(PathBuf::from("item.bin"), |file| file.write_all(b"new"))
        .unwrap();
    assert_eq!(std::fs::read(&outside).unwrap(), b"outside sentinel");
    assert_eq!(std::fs::read(&backup).unwrap(), b"old");
    assert_eq!(std::fs::read(&destination).unwrap(), b"new");
    assert!(
        !std::fs::symlink_metadata(&destination)
            .unwrap()
            .file_type()
            .is_symlink()
    );
}

#[test]
fn concurrent_writers_publish_exactly_one_immutable_output() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    std::fs::create_dir_all(temp.path().join("ab/cd")).unwrap();
    let barrier = Arc::new(Barrier::new(16));
    let writes = std::thread::scope(|scope| {
        let handles = (0_u8..16)
            .map(|value| {
                let writer = writer.clone();
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    writer
                        .write_atomic(PathBuf::from("ab/cd/race.bin"), |file| {
                            file.write_all(&[value])?;
                            barrier.wait();
                            Ok(())
                        })
                        .unwrap()
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert_eq!(writes.iter().filter(|write| !write.reused).count(), 1);
    assert_eq!(
        std::fs::read(temp.path().join("ab/cd/race.bin"))
            .unwrap()
            .len(),
        1
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
fn replacement_failure_preserves_existing_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    writer
        .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
            file.write_all(b"old")
        })
        .unwrap();
    writer.fail_next_replace_for_test();

    let result = writer.replace_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
        file.write_all(b"new")
    });

    assert!(result.is_err());
    assert_eq!(
        std::fs::read(temp.path().join("ab/cd/item.bin")).unwrap(),
        b"old"
    );
    assert!(partial_files(temp.path()).is_empty());
}

#[test]
fn cleanup_failure_is_reported_without_removing_the_managed_file() {
    let temp = tempfile::tempdir().unwrap();
    let writer = CacheWriter::new(temp.path()).unwrap();
    writer
        .write_atomic(PathBuf::from("item.bin"), |file| file.write_all(b"managed"))
        .unwrap();
    writer.fail_next_cleanup_for_test();

    assert!(matches!(
        writer.remove_checked(std::path::Path::new("item.bin")),
        Err(CacheError::Io(_))
    ));
    assert_eq!(
        std::fs::read(temp.path().join("item.bin")).unwrap(),
        b"managed"
    );
}

#[cfg(unix)]
#[test]
fn atomic_write_pins_ancestor_for_create_and_replace() {
    for replace in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let cache = temp.path().join("cache");
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let writer = CacheWriter::new(&cache).unwrap();
        if replace {
            writer
                .write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
                    file.write_all(b"old")
                })
                .unwrap();
        }
        std::fs::write(outside.join("item.bin"), b"outside").unwrap();

        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let hook_entered = entered.clone();
        let hook_release = release.clone();
        writer.install_path_race_test_hook(Arc::new(move || {
            hook_entered.wait();
            hook_release.wait();
        }));
        let worker = writer.clone();
        let operation = thread::spawn(move || {
            if replace {
                worker.replace_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
                    file.write_all(b"new")
                })
            } else {
                worker.write_atomic(PathBuf::from("ab/cd/item.bin"), |file| {
                    file.write_all(b"new")
                })
            }
        });
        entered.wait();
        std::fs::rename(cache.join("ab"), cache.join("ab-moved")).unwrap();
        std::os::unix::fs::symlink(&outside, cache.join("ab")).unwrap();
        release.wait();
        operation.join().unwrap().unwrap();

        assert_eq!(
            std::fs::read(cache.join("ab-moved/cd/item.bin")).unwrap(),
            b"new"
        );
        assert_eq!(std::fs::read(outside.join("item.bin")).unwrap(), b"outside");
    }
}

#[cfg(unix)]
#[test]
fn atomic_write_rejects_a_final_component_swap_without_touching_outside() {
    let temp = tempfile::tempdir().unwrap();
    let cache = temp.path().join("cache");
    let outside = temp.path().join("outside.bin");
    std::fs::write(&outside, b"outside").unwrap();
    let writer = CacheWriter::new(&cache).unwrap();

    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let hook_entered = entered.clone();
    let hook_release = release.clone();
    writer.install_path_race_test_hook(Arc::new(move || {
        hook_entered.wait();
        hook_release.wait();
    }));
    let worker = writer.clone();
    let write = thread::spawn(move || {
        worker.write_atomic(PathBuf::from("item.bin"), |file| file.write_all(b"managed"))
    });
    entered.wait();
    std::os::unix::fs::symlink(&outside, cache.join("item.bin")).unwrap();
    release.wait();

    assert!(matches!(write.join().unwrap(), Err(CacheError::PathEscape)));
    assert_eq!(std::fs::read(outside).unwrap(), b"outside");
    assert!(partial_files(&cache).is_empty());
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
fn catalog_rejects_an_escaping_derivative_path() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = CacheFixture::new(temp.path());
    let group = fixture.group("collection", Some(1));

    let result = fixture.catalog.insert_derivative(&NewDerivative {
        id: DerivativeId::new(),
        asset_id: fixture.asset_id,
        folder_group_id: group,
        kind: "screen_preview".to_owned(),
        cache_key: "escape".to_owned(),
        relative_cache_path: PathBuf::from("../source-original.jpg"),
        size_bytes: 6,
        durable: false,
        created_at: 1,
    });

    assert!(matches!(result, Err(CatalogError::InvalidData(_))));
    assert_eq!(fixture.catalog.derivative_count(group, false).unwrap(), 0);
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

    EvictionPlanner::execute(
        &mut fixture.catalog,
        temp.path(),
        &plan,
        &ProtectedGroups::default(),
    )
    .unwrap();
    assert_eq!(fixture.catalog.derivative_count(old, false).unwrap(), 0);
    assert_eq!(fixture.catalog.derivative_count(old, true).unwrap(), 1);
    assert_eq!(fixture.catalog.derivative_count(new, false).unwrap(), 1);
    assert!(!temp.path().join("old-preview.bin").exists());
    assert!(temp.path().join("old-thumb.bin").exists());
}

#[test]
fn shared_derivative_bytes_are_counted_only_when_all_owners_are_evicted() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = CacheFixture::new(temp.path());
    let first = fixture.group("first", Some(10));
    let second = fixture.group("second", Some(20));
    fixture.derivative(first, "shared", 40, false);
    let derivative = fixture.catalog.all_derivatives().unwrap()[0].id;
    fixture
        .catalog
        .link_derivative_group(derivative, second)
        .unwrap();

    let plan = EvictionPlanner::plan(&fixture.catalog, 35, &ProtectedGroups::default()).unwrap();

    assert_eq!(plan.groups, vec![first, second]);
    assert_eq!(plan.reclaimable_bytes, 40);
    EvictionPlanner::execute(
        &mut fixture.catalog,
        temp.path(),
        &plan,
        &ProtectedGroups::default(),
    )
    .unwrap();
    assert_eq!(fixture.catalog.all_derivatives().unwrap().len(), 0);
    assert!(!temp.path().join("shared.bin").exists());
}

#[test]
fn eviction_transaction_blocks_a_cross_connection_group_link_until_commit() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&database).unwrap();
    let library = NewLibrary::configured("Photos", temp.path());
    let library_id = catalog.add_library(&library).unwrap().id;
    let first = fixture_group(&mut catalog, library_id, "first");
    let second = fixture_group(&mut catalog, library_id, "second");
    let asset = photo_catalog::NewAsset::minimal(
        library_id,
        RelativePathKey::from_relative_path(std::path::Path::new("image.jpg")).unwrap(),
        "image.jpg",
        MediaKind::Jpeg,
        1,
    );
    catalog.upsert_asset(&asset).unwrap();
    fixture_derivative(&mut catalog, first, asset.id, "shared", 4);
    let derivative = catalog.all_derivatives().unwrap()[0].id;

    let eviction = catalog.begin_derivative_eviction(&[first]).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let other_database = database.clone();
    let handle = thread::spawn(move || {
        let mut other = Catalog::open(&other_database).unwrap();
        started_tx.send(()).unwrap();
        let linked = other.link_derivative_group(derivative, second).is_ok();
        done_tx.send(linked).unwrap();
    });
    started_rx.recv().unwrap();
    assert!(done_rx.recv_timeout(Duration::from_millis(50)).is_err());

    eviction.commit().unwrap();
    assert!(!done_rx.recv_timeout(Duration::from_secs(2)).unwrap());
    handle.join().unwrap();
    let reopened = Catalog::open(&database).unwrap();
    assert_eq!(reopened.derivative_count(second, false).unwrap(), 0);
}

fn fixture_group(catalog: &mut Catalog, library_id: LibraryId, path: &str) -> FolderGroupId {
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: FolderGroupId::new(),
            library_id,
            relative_path: RelativePathKey::from_relative_path(std::path::Path::new(path)).unwrap(),
            display_path: path.to_owned(),
            last_viewed_at: Some(1),
        })
        .unwrap()
}

fn fixture_derivative(
    catalog: &mut Catalog,
    group: FolderGroupId,
    asset_id: AssetId,
    cache_key: &str,
    size_bytes: u64,
) {
    catalog
        .insert_derivative(&NewDerivative {
            id: DerivativeId::new(),
            asset_id,
            folder_group_id: group,
            kind: "screen_preview".to_owned(),
            cache_key: cache_key.to_owned(),
            relative_cache_path: PathBuf::from(format!("{cache_key}.bin")),
            size_bytes,
            durable: false,
            created_at: 1,
        })
        .unwrap();
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
    let groups = ProtectedGroups::default();
    groups.protect(protected).unwrap();
    let _active_write = groups.begin_write(active).unwrap();

    let plan = EvictionPlanner::plan(&fixture.catalog, 10, &groups).unwrap();

    assert_eq!(plan.groups, vec![eligible]);
}

#[test]
fn eviction_rechecks_activity_after_planning() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = CacheFixture::new(temp.path());
    let group = fixture.group("collection", Some(1));
    fixture.derivative(group, "preview", 20, false);
    let groups = ProtectedGroups::default();
    let plan = EvictionPlanner::plan(&fixture.catalog, 10, &groups).unwrap();
    let _active_write = groups.begin_write(group).unwrap();

    let result = EvictionPlanner::execute(&mut fixture.catalog, temp.path(), &plan, &groups);

    assert!(matches!(result, Err(CacheError::GroupBecameProtected)));
    assert!(temp.path().join("preview.bin").exists());
    assert_eq!(fixture.catalog.derivative_count(group, false).unwrap(), 1);
}

#[test]
fn zero_byte_eviction_request_selects_no_groups() {
    let temp = tempfile::tempdir().unwrap();
    let mut fixture = CacheFixture::new(temp.path());
    let group = fixture.group("collection", Some(1));
    fixture.derivative(group, "preview", 20, false);

    let plan = EvictionPlanner::plan(&fixture.catalog, 0, &ProtectedGroups::default()).unwrap();

    assert!(plan.groups.is_empty());
    assert_eq!(plan.reclaimable_bytes, 0);
}

#[cfg(unix)]
#[test]
fn eviction_rejects_symlinked_cache_ancestors_before_deleting() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("source-original.jpg"), b"source").unwrap();
    symlink(outside.path(), temp.path().join("linked")).unwrap();
    let mut fixture = CacheFixture::new(temp.path());
    let group = fixture.group("collection", Some(1));
    fixture.derivative_row_at(
        group,
        "symlink-row",
        PathBuf::from("linked/source-original.jpg"),
        6,
        false,
    );
    let plan = EvictionPlanner::plan(&fixture.catalog, 1, &ProtectedGroups::default()).unwrap();

    let result = EvictionPlanner::execute(
        &mut fixture.catalog,
        temp.path(),
        &plan,
        &ProtectedGroups::default(),
    );

    assert!(matches!(result, Err(CacheError::PathEscape)));
    assert_eq!(
        std::fs::read(outside.path().join("source-original.jpg")).unwrap(),
        b"source"
    );
    assert_eq!(fixture.catalog.derivative_count(group, false).unwrap(), 1);
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
            .unwrap()
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
        self.derivative_row_at(
            group,
            name,
            PathBuf::from(format!("{name}.bin")),
            bytes,
            durable,
        );
    }

    fn derivative_row_at(
        &mut self,
        group: FolderGroupId,
        name: &str,
        relative_cache_path: PathBuf,
        bytes: u64,
        durable: bool,
    ) {
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
                relative_cache_path,
                size_bytes: bytes,
                durable,
                created_at: 1,
            })
            .unwrap();
    }
}
