use std::path::Path;

use photo_catalog::{Catalog, CatalogError, NewFolderGroup, NewLibrary};
use photo_domain::{Availability, FolderGroupId, LibraryId, RelativePathKey};
use rusqlite::{Connection, params};

fn add_group(catalog: &mut Catalog, library: LibraryId, path: &str) -> FolderGroupId {
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

#[test]
fn root_and_group_outages_persist_checked_recovery_tokens() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
        .unwrap();
    let parent = add_group(&mut catalog, library.id, "");
    let child = add_group(&mut catalog, library.id, "child");

    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, parent)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 0,
            reconciled: 0,
        }
    );
    catalog.mark_root_offline(library.id).unwrap();
    let added_while_offline = add_group(&mut catalog, library.id, "offline-empty");
    for group in [parent, child] {
        assert_eq!(
            catalog
                .folder_group_recovery_state(library.id, group)
                .unwrap(),
            photo_catalog::FolderGroupRecoveryState {
                requested: 1,
                reconciled: 0,
            }
        );
    }
    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, added_while_offline)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 1,
            reconciled: 0,
        },
        "an empty group created during an outage must inherit the pending recovery"
    );
    catalog.mark_root_offline(library.id).unwrap();
    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, child)
            .unwrap()
            .requested,
        1,
        "one root outage must request one recovery"
    );

    catalog
        .set_library_availability(library.id, Availability::Available)
        .unwrap();
    let generation = catalog
        .begin_generation_for_group_at_recovery(library.id, child, 1)
        .unwrap();
    assert!(
        catalog
            .complete_generation_for_group_at_recovery(library.id, child, generation, 0)
            .is_err(),
        "completion must reject a token that differs from the scan row"
    );
    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, child)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 1,
            reconciled: 0,
        }
    );
    catalog
        .complete_generation_for_group_at_recovery(library.id, child, generation, 1)
        .unwrap();
    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, child)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 1,
            reconciled: 1,
        }
    );

    catalog.mark_group_offline(library.id, child).unwrap();
    catalog.mark_group_offline(library.id, child).unwrap();
    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, child)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 2,
            reconciled: 1,
        },
        "a repeated group-local outage must not create a scan storm"
    );
    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, parent)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 1,
            reconciled: 0,
        },
        "group-local recovery must remain isolated"
    );
}

#[test]
fn newer_outage_during_recovery_remains_pending_after_the_old_scan_finishes() {
    let mut catalog = Catalog::open_in_memory().unwrap();
    let library = catalog
        .add_library(&NewLibrary::configured("Photos", Path::new("/Photos")))
        .unwrap();
    let group = add_group(&mut catalog, library.id, "");

    catalog.mark_root_offline(library.id).unwrap();
    catalog
        .set_library_availability(library.id, Availability::Available)
        .unwrap();
    let first = catalog
        .begin_generation_for_group_at_recovery(library.id, group, 1)
        .unwrap();

    catalog.mark_root_offline(library.id).unwrap();
    catalog
        .set_library_availability(library.id, Availability::Available)
        .unwrap();
    catalog
        .complete_generation_for_group_at_recovery(library.id, group, first, 1)
        .unwrap();
    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, group)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 2,
            reconciled: 0,
        },
        "the first scan was marked offline and cannot consume the newer request"
    );

    let second = catalog
        .begin_generation_for_group_at_recovery(library.id, group, 2)
        .unwrap();
    catalog
        .complete_generation_for_group_at_recovery(library.id, group, second, 2)
        .unwrap();
    assert_eq!(
        catalog
            .folder_group_recovery_state(library.id, group)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 2,
            reconciled: 2,
        }
    );
}

#[test]
fn recovery_counter_overflow_rolls_back_root_and_group_outages() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let mut catalog = Catalog::open(&path).unwrap();
    let root_library = catalog
        .add_library(&NewLibrary::configured("Root", Path::new("/root")))
        .unwrap();
    let root_group = add_group(&mut catalog, root_library.id, "");
    let group_library = catalog
        .add_library(&NewLibrary::configured("Group", Path::new("/group")))
        .unwrap();
    let group = add_group(&mut catalog, group_library.id, "");
    drop(catalog);

    let connection = Connection::open(&path).unwrap();
    connection
        .execute(
            "UPDATE folder_groups SET recovery_requested = ?2 WHERE id = ?1",
            params![root_group.as_uuid().as_bytes(), i64::MAX],
        )
        .unwrap();
    connection
        .execute(
            "UPDATE folder_groups
             SET recovery_requested = ?2, recovery_reconciled = ?2 WHERE id = ?1",
            params![group.as_uuid().as_bytes(), i64::MAX],
        )
        .unwrap();
    drop(connection);

    let mut catalog = Catalog::open(&path).unwrap();
    assert!(matches!(
        catalog.mark_root_offline(root_library.id),
        Err(CatalogError::ValueOutOfRange)
    ));
    assert_eq!(
        catalog
            .find_library(root_library.id)
            .unwrap()
            .unwrap()
            .availability,
        Availability::Available,
        "overflow must roll back the root availability transition"
    );
    assert!(matches!(
        catalog.mark_group_offline(group_library.id, group),
        Err(CatalogError::ValueOutOfRange)
    ));
    assert_eq!(
        catalog
            .folder_group_recovery_state(group_library.id, group)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: i64::MAX as u64,
            reconciled: i64::MAX as u64,
        }
    );
}

#[test]
fn v9_migration_selectively_backfills_durable_offline_evidence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("catalog.sqlite");
    let clean_library = LibraryId::new();
    let root_offline_library = LibraryId::new();
    let group_offline_library = LibraryId::new();
    let clean_group = FolderGroupId::new();
    let root_offline_empty_group = FolderGroupId::new();
    let group_offline_group = FolderGroupId::new();
    let asset_path = RelativePathKey::from_relative_path(Path::new("offline/photo.jpg")).unwrap();
    let asset_id = photo_domain::AssetId::for_path(group_offline_library, &asset_path);
    {
        let connection = Connection::open(&path).unwrap();
        for migration in [
            include_str!("../migrations/0001_catalog.sql"),
            include_str!("../migrations/0002_unavailable_assets.sql"),
            include_str!("../migrations/0003_app_state.sql"),
            include_str!("../migrations/0004_wall_projection.sql"),
            include_str!("../migrations/0005_group_scoped_generations.sql"),
            include_str!("../migrations/0006_wall_state_indexes.sql"),
            include_str!("../migrations/0007_derivative_coordinator.sql"),
            include_str!("../migrations/0008_gallery_scope.sql"),
            include_str!("../migrations/0009_selection_membership.sql"),
        ] {
            connection.execute_batch(migration).unwrap();
        }
        for (library, name, root, availability) in [
            (clean_library, "Clean", b"/clean".as_slice(), "available"),
            (
                root_offline_library,
                "Root offline",
                b"/root-offline".as_slice(),
                "root_offline",
            ),
            (
                group_offline_library,
                "Group offline",
                b"/group-offline".as_slice(),
                "available",
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO library_roots
                     (id, kind, display_name, canonical_root_key, display_path, availability)
                     VALUES (?1, 'configured', ?2, ?3, ?2, ?4)",
                    params![library.as_uuid().as_bytes(), name, root, availability],
                )
                .unwrap();
        }
        for (group, library, path) in [
            (clean_group, clean_library, b"".as_slice()),
            (
                root_offline_empty_group,
                root_offline_library,
                b"empty".as_slice(),
            ),
            (
                group_offline_group,
                group_offline_library,
                b"offline".as_slice(),
            ),
        ] {
            connection
                .execute(
                    "INSERT INTO folder_groups
                     (id, library_id, relative_path_key, display_path)
                     VALUES (?1, ?2, ?3, 'group')",
                    params![
                        group.as_uuid().as_bytes(),
                        library.as_uuid().as_bytes(),
                        path
                    ],
                )
                .unwrap();
        }
        connection
            .execute(
                "INSERT INTO assets
                 (id, library_id, folder_group_id, relative_path_key, display_path,
                  media_kind, size_bytes, modified_unix_ns, visible_by_default,
                  availability, last_seen_generation)
                 VALUES (?1, ?2, ?3, 'offline/photo.jpg', 'offline/photo.jpg',
                         'jpeg', 1, '1', 1, 'root_offline', 0)",
                params![
                    asset_id.as_uuid().as_bytes(),
                    group_offline_library.as_uuid().as_bytes(),
                    group_offline_group.as_uuid().as_bytes(),
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO scan_generations
                 (library_id, folder_group_id, generation, started_at, source_was_online)
                 VALUES (?1, ?2, 1, 0, 1)",
                params![
                    clean_library.as_uuid().as_bytes(),
                    clean_group.as_uuid().as_bytes()
                ],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO folder_group_assets
                 (folder_group_id, asset_id, last_seen_generation) VALUES (?1, ?2, 0)",
                params![
                    group_offline_group.as_uuid().as_bytes(),
                    asset_id.as_uuid().as_bytes()
                ],
            )
            .unwrap();
    }

    let mut catalog = Catalog::open(&path).unwrap();
    assert_eq!(
        catalog
            .folder_group_recovery_state(clean_library, clean_group)
            .unwrap(),
        photo_catalog::FolderGroupRecoveryState {
            requested: 0,
            reconciled: 0,
        }
    );
    for (library, group) in [
        (root_offline_library, root_offline_empty_group),
        (group_offline_library, group_offline_group),
    ] {
        assert_eq!(
            catalog.folder_group_recovery_state(library, group).unwrap(),
            photo_catalog::FolderGroupRecoveryState {
                requested: 1,
                reconciled: 0,
            }
        );
    }
    catalog
        .complete_generation_for_group(clean_library, clean_group, 1)
        .expect("a migrated token-zero generation remains desktop-compatible");
}
