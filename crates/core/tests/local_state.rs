#[cfg(feature = "server-internal-prevalidated-source")]
use photo_core::PrevalidatedSourceKeys;
#[cfg(target_os = "macos")]
use photo_core::normalize_prevalidated_source_key;
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

#[cfg(feature = "server-internal-prevalidated-source")]
#[test]
fn prevalidated_missing_source_keys_are_checked_without_resolving_the_source() {
    let temp = tempfile::tempdir().unwrap();
    let missing_source = temp.path().join("missing-photos");
    let state = LocalStatePaths::new(
        missing_source.join("data"),
        temp.path().join("cache-not-created"),
    );

    // SAFETY: these absolute paths are the explicit identity keys under test;
    // no filesystem identity is inferred from them by this fixture.
    let missing_keys = unsafe {
        PrevalidatedSourceKeys::from_validated_identity_keys(vec![missing_source.clone()])
    }
    .unwrap();
    assert!(matches!(
        state.prepare_prevalidated_source_keys(&missing_keys),
        Err(LocalStateError::InsideSourceRoot)
    ));
    assert!(!missing_source.exists());

    let disjoint_source = temp.path().join("other-missing-photos");
    let disjoint = LocalStatePaths::new(temp.path().join("data"), temp.path().join("cache"));
    // SAFETY: same fixture-scoped identity-key invariant as above.
    let disjoint_keys =
        unsafe { PrevalidatedSourceKeys::from_validated_identity_keys(vec![disjoint_source]) }
            .unwrap();
    assert!(
        disjoint
            .validate_prevalidated_source_keys(&disjoint_keys)
            .is_ok()
    );
}

#[cfg(feature = "server-internal-prevalidated-source")]
#[test]
fn prevalidated_source_key_capability_rejects_relative_inputs() {
    // SAFETY: deliberately supplies an invalid key to exercise the runtime
    // validation performed before a capability can be returned.
    let result = unsafe {
        PrevalidatedSourceKeys::from_validated_identity_keys(vec!["relative/photos".into()])
    };

    let error = result.unwrap_err();
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert_eq!(error.to_string(), "prevalidated source key is invalid");
}

#[cfg(target_os = "macos")]
#[test]
fn prevalidated_keys_normalize_only_exact_builtin_macos_aliases() {
    assert_eq!(
        normalize_prevalidated_source_key(std::path::Path::new("/var/photos/../family")).unwrap(),
        std::path::Path::new("/private/var/family")
    );
    assert_eq!(
        normalize_prevalidated_source_key(std::path::Path::new("/various/family")).unwrap(),
        std::path::Path::new("/various/family")
    );
}
