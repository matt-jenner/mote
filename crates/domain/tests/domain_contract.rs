use std::path::Path;

use photo_domain::{AssetId, LibraryId, MediaKind, RelativePathKey};

#[test]
fn asset_identity_is_stable_for_library_and_native_relative_path() {
    let library = LibraryId::from_uuid(uuid::Uuid::from_u128(1));
    let path = RelativePathKey::from_relative_path(Path::new("2026/Trip/a.raw")).unwrap();

    let first = AssetId::for_path(library, &path);
    let second = AssetId::for_path(library, &path);

    assert_eq!(first, second);
}

#[test]
fn relative_path_key_rejects_absolute_paths() {
    let absolute = if cfg!(windows) {
        r"C:\Photos\a.jpg"
    } else {
        "/Photos/a.jpg"
    };

    assert!(RelativePathKey::from_relative_path(Path::new(absolute)).is_err());
}

#[test]
fn relative_path_key_round_trips_unicode_names() {
    let path = Path::new("2026/Été/東京.jpg");

    let key = RelativePathKey::from_relative_path(path).unwrap();

    assert_eq!(key.to_path_buf().unwrap(), path);
}

#[cfg(unix)]
#[test]
fn relative_path_key_round_trips_non_utf8_names() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let path = Path::new(OsStr::from_bytes(b"2026/raw/\xFF.cr3"));

    let key = RelativePathKey::from_relative_path(path).unwrap();

    assert_eq!(key.to_path_buf().unwrap(), path);
}

#[test]
fn classifies_supported_extensions_case_insensitively() {
    assert_eq!(
        MediaKind::from_path(Path::new("a.CR3")),
        Some(MediaKind::Raw)
    );
    assert_eq!(
        MediaKind::from_path(Path::new("clip.MP4")),
        Some(MediaKind::Video)
    );
    assert_eq!(MediaKind::from_path(Path::new("notes.txt")), None);
}
