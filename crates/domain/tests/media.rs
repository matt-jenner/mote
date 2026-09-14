use std::path::Path;

use photo_domain::MediaKind;

#[test]
fn heif_extensions_are_recognized_case_insensitively_in_every_build() {
    assert_eq!(
        MediaKind::from_path(Path::new("phone.HEIC")),
        Some(MediaKind::Heif)
    );
    assert_eq!(
        MediaKind::from_path(Path::new("phone.HEIF")),
        Some(MediaKind::Heif)
    );
}

fn assert_non_heif_wall_capabilities() {
    for kind in [
        MediaKind::Jpeg,
        MediaKind::Png,
        MediaKind::Tiff,
        MediaKind::Webp,
    ] {
        assert!(kind.is_wall_viewable(), "{kind:?} should remain viewable");
    }
    for kind in [
        MediaKind::Avif,
        MediaKind::Raw,
        MediaKind::Video,
        MediaKind::Unknown,
    ] {
        assert!(!kind.is_wall_viewable(), "{kind:?} must remain excluded");
    }
}

#[cfg(feature = "heic")]
#[test]
fn enabled_build_admits_heif_to_the_wall_capability() {
    assert_non_heif_wall_capabilities();
    assert!(MediaKind::Heif.is_wall_viewable());
}

#[cfg(not(feature = "heic"))]
#[test]
fn disabled_build_keeps_heif_recognized_but_not_wall_viewable() {
    assert_non_heif_wall_capabilities();
    assert!(!MediaKind::Heif.is_wall_viewable());
}
