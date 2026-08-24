use photo_core::{LocalStateError, LocalStatePaths};

#[test]
fn validation_happens_before_any_local_directory_is_created() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    std::fs::create_dir(&source).unwrap();
    let state = LocalStatePaths::new(
        source.join("app-data"),
        temp.path().join("cache-not-created"),
    );

    assert!(matches!(
        state.prepare(std::slice::from_ref(&source)),
        Err(LocalStateError::InsideSourceRoot)
    ));
    assert!(!state.cache_dir().exists());
}

#[cfg(unix)]
#[test]
fn symlink_aliases_are_resolved_before_overlap_checks() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photos");
    let alias = temp.path().join("photos-alias");
    std::fs::create_dir(&source).unwrap();
    symlink(&source, &alias).unwrap();
    let state = LocalStatePaths::new(alias.join("data"), temp.path().join("cache"));

    assert!(matches!(
        state.validate_source_roots(&[source]),
        Err(LocalStateError::InsideSourceRoot)
    ));
}

#[cfg(unix)]
#[test]
fn prepared_directories_are_private() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().unwrap();
    let state = LocalStatePaths::new(temp.path().join("data"), temp.path().join("cache"));
    state.prepare(&[]).unwrap();

    for path in [state.data_dir(), state.cache_dir()] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
