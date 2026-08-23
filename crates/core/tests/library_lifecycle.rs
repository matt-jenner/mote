use std::path::{Path, PathBuf};

use photo_catalog::Catalog;
use photo_core::{AddLibraryError, LibraryService, RealSourceFs, RelinkError};
use photo_domain::{Availability, LibraryKind, RelativePathKey};

fn service() -> LibraryService<RealSourceFs> {
    LibraryService::new(Catalog::open_in_memory().unwrap(), RealSourceFs, Vec::new()).unwrap()
}

fn relative(path: &str) -> RelativePathKey {
    RelativePathKey::from_relative_path(Path::new(path)).unwrap()
}

#[test]
fn folder_inside_existing_library_becomes_a_selection_not_a_second_root() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Photos");
    std::fs::create_dir_all(root.join("2026/Trip")).unwrap();
    let mut service = service();
    let library = service.add_configured(&root, "Photos").unwrap();

    let selected = service.open_recent(&root.join("2026/Trip")).unwrap();

    assert_eq!(selected.library_id, library.id);
    assert_eq!(
        selected.relative_folder.to_path_buf().unwrap(),
        PathBuf::from("2026/Trip")
    );
    assert!(!selected.created_recent_root);
    assert_eq!(service.catalog().list_libraries().unwrap().len(), 1);
}

#[test]
fn configured_roots_cannot_overlap_existing_roots() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Photos");
    std::fs::create_dir_all(root.join("2026/Trip")).unwrap();
    let mut service = service();
    let library = service.add_configured(&root, "Photos").unwrap();

    for overlapping in [root.clone(), root.join("2026"), temp.path().to_path_buf()] {
        assert!(matches!(
            service.add_configured(&overlapping, "Overlap"),
            Err(AddLibraryError::Overlaps { existing_id }) if existing_id == library.id
        ));
    }
}

#[test]
fn sources_and_relinks_cannot_overlap_local_state_roots() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("Photos/.photo-viewer-state");
    let old_root = temp.path().join("Old Photos");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir(&old_root).unwrap();
    let mut service = LibraryService::new(
        Catalog::open_in_memory().unwrap(),
        RealSourceFs,
        vec![state.canonicalize().unwrap()],
    )
    .unwrap();

    assert!(matches!(
        service.add_configured(temp.path().join("Photos").as_path(), "Unsafe"),
        Err(AddLibraryError::OverlapsLocalState)
    ));

    let library = service.add_configured(&old_root, "Old").unwrap();
    assert!(matches!(
        service.relink(library.id, temp.path().join("Photos").as_path(), &[]),
        Err(RelinkError::OverlapsLocalState)
    ));
}

#[cfg(unix)]
#[test]
fn local_state_aliases_are_canonicalized_before_add_and_relink() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let photos = temp.path().join("physical/Photos");
    let state = photos.join(".photo-viewer-state");
    let state_alias = temp.path().join("state-alias");
    let old_root = temp.path().join("Old Photos");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir(&old_root).unwrap();
    symlink(&state, &state_alias).unwrap();
    let mut service = LibraryService::new(
        Catalog::open_in_memory().unwrap(),
        RealSourceFs,
        vec![state_alias],
    )
    .unwrap();

    assert!(matches!(
        service.add_configured(&photos, "Unsafe"),
        Err(AddLibraryError::OverlapsLocalState)
    ));

    let library = service.add_configured(&old_root, "Old").unwrap();
    assert!(matches!(
        service.relink(library.id, &photos, &[]),
        Err(RelinkError::OverlapsLocalState)
    ));
}

#[test]
fn standalone_folder_starts_recent_and_can_be_promoted() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("Loose Photos");
    std::fs::create_dir_all(&root).unwrap();
    let mut service = service();

    let selection = service.open_recent(&root).unwrap();
    let promoted = service
        .promote_recent(selection.library_id, "Archive")
        .unwrap();

    assert!(selection.created_recent_root);
    assert_eq!(promoted.kind, LibraryKind::Configured);
    assert_eq!(promoted.display_name, "Archive");
}

#[test]
fn relink_preserves_library_id_after_sample_verification() {
    let temp = tempfile::tempdir().unwrap();
    let old_root = temp.path().join("old");
    let new_root = temp.path().join("new");
    for root in [&old_root, &new_root] {
        std::fs::create_dir_all(root.join("Processed")).unwrap();
        std::fs::write(root.join("a.jpg"), b"a").unwrap();
        std::fs::write(root.join("Processed/b.jpg"), b"b").unwrap();
    }
    let mut service = service();
    let library = service.add_configured(&old_root, "Photos").unwrap();
    service.mark_offline(library.id).unwrap();
    let samples = [relative("a.jpg"), relative("Processed/b.jpg")];

    let relinked = service.relink(library.id, &new_root, &samples).unwrap();

    assert_eq!(relinked.id, library.id);
    assert_eq!(relinked.display_path, new_root.to_string_lossy());
    assert_eq!(relinked.availability, Availability::Available);

    std::fs::remove_file(new_root.join("Processed/b.jpg")).unwrap();
    assert!(matches!(
        service.relink(library.id, &new_root, &samples),
        Err(RelinkError::VerificationFailed { .. })
    ));
}
