use photo_cache::{DerivativeKind, DerivativeSpec, DerivativeTarget, ImageDerivativeGenerator};
use photo_domain::{AssetId, FileSignature, LibraryId};

fn spec(kind: DerivativeKind, edge: u32, orientation: u16) -> DerivativeSpec {
    DerivativeSpec {
        asset_id: AssetId::for_path(
            LibraryId::from_uuid(uuid::Uuid::from_u128(1)),
            &photo_domain::RelativePathKey::from_relative_path(std::path::Path::new("x.jpg"))
                .unwrap(),
        ),
        signature: FileSignature {
            size_bytes: 1,
            modified_unix_ns: 1,
            sidecar_modified_unix_ns: None,
        },
        orientation,
        kind,
        decoder_version: "image-0.25-v1".into(),
        colour_space: "srgb".into(),
        target: DerivativeTarget::LongEdge(edge),
    }
}

fn fixture() -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
    let image = image::RgbImage::from_fn(1200, 800, |x, y| {
        image::Rgb([(x % 255) as u8, (y % 255) as u8, 20])
    });
    image
        .save_with_format(file.path(), image::ImageFormat::Jpeg)
        .unwrap();
    file
}

#[test]
fn derivative_generator_api_is_available() {
    let temp = tempfile::tempdir().unwrap();
    let _ = ImageDerivativeGenerator::new(temp.path());
    let _ = spec(DerivativeKind::WallThumbnail, 1024, 1);
}

#[test]
fn wall_and_screen_outputs_respect_long_edge_and_orientation() {
    let fixture = fixture();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let wall = generator
        .generate(
            fixture.path(),
            &spec(DerivativeKind::WallThumbnail, 1024, 6),
        )
        .unwrap();
    let screen = generator
        .generate(
            fixture.path(),
            &spec(DerivativeKind::ScreenPreview, 4096, 6),
        )
        .unwrap();
    let wall_size = image::image_dimensions(cache.path().join(&wall.relative_path)).unwrap();
    let screen_size = image::image_dimensions(cache.path().join(&screen.relative_path)).unwrap();
    assert_eq!(wall_size, (683, 1024));
    assert_eq!(screen_size, (800, 1200));
    assert!(wall.durable);
    assert!(!screen.durable);
}

#[test]
fn a_second_identical_request_reuses_the_atomic_cache_file_and_source() {
    let fixture = fixture();
    let before = std::fs::read(fixture.path()).unwrap();
    let modified = std::fs::metadata(fixture.path())
        .unwrap()
        .modified()
        .unwrap();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let requested = spec(DerivativeKind::WallThumbnail, 1024, 1);
    let first = generator.generate(fixture.path(), &requested).unwrap();
    let screen = generator
        .generate(
            fixture.path(),
            &spec(DerivativeKind::ScreenPreview, 4096, 1),
        )
        .unwrap();
    let second = generator.generate(fixture.path(), &requested).unwrap();
    assert_eq!(
        (second.key, second.relative_path),
        (first.key, first.relative_path)
    );
    assert!(second.reused);
    assert_eq!(screen.content_type, "image/jpeg");
    assert_eq!(screen.representative_rgb, first.representative_rgb);
    assert_eq!(std::fs::read(fixture.path()).unwrap(), before);
    assert_eq!(
        std::fs::metadata(fixture.path())
            .unwrap()
            .modified()
            .unwrap(),
        modified
    );
}

#[test]
fn all_exif_orientations_normalize_dimensions() {
    let fixture = fixture();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    for orientation in 1..=8 {
        let result = generator
            .generate(
                fixture.path(),
                &spec(DerivativeKind::ScreenPreview, 4096, orientation),
            )
            .unwrap();
        let dimensions = image::image_dimensions(cache.path().join(result.relative_path)).unwrap();
        if orientation >= 5 {
            assert_eq!(dimensions, (800, 1200), "orientation {orientation}");
        } else {
            assert_eq!(dimensions, (1200, 800), "orientation {orientation}");
        }
        assert_eq!(result.content_type, "image/jpeg");
    }
}
