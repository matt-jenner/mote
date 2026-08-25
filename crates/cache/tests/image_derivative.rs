use photo_cache::{
    CacheBudget, DerivativeKind, DerivativeSpec, DerivativeTarget, ImageDerivativeError,
    ImageDerivativeGenerator, ProtectedGroups,
};
use photo_catalog::{Catalog, NewFolderGroup, NewLibrary};
use photo_domain::{AssetId, FileSignature, FolderGroupId, LibraryId, MediaKind, RelativePathKey};

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

fn corner_fixture() -> tempfile::NamedTempFile {
    let file = tempfile::Builder::new().suffix(".jpg").tempfile().unwrap();
    let image = image::RgbImage::from_fn(80, 60, |x, y| match (x < 40, y < 30) {
        (true, true) => image::Rgb([220, 20, 20]),
        (false, true) => image::Rgb([20, 210, 20]),
        (true, false) => image::Rgb([20, 30, 220]),
        (false, false) => image::Rgb([220, 210, 20]),
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
    let wall_size = image::image_dimensions(cache.path().join(&wall.relative_path)).unwrap();
    assert_eq!(wall_size, (683, 1024));
    assert!(wall.durable);
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
    assert!(matches!(
        generator.generate(
            fixture.path(),
            &spec(DerivativeKind::ScreenPreview, 4096, 1)
        ),
        Err(photo_cache::ImageDerivativeError::BudgetAuthorizationRequired)
    ));
    let second = generator.generate(fixture.path(), &requested).unwrap();
    assert_eq!(
        (second.key, second.relative_path),
        (first.key, first.relative_path)
    );
    assert!(second.reused);
    assert_eq!(first.content_type, "image/jpeg");
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
                &spec(DerivativeKind::WallThumbnail, 1024, orientation),
            )
            .unwrap();
        let dimensions = image::image_dimensions(cache.path().join(result.relative_path)).unwrap();
        if orientation >= 5 {
            assert_eq!(dimensions, (683, 1024), "orientation {orientation}");
        } else {
            assert_eq!(dimensions, (1024, 683), "orientation {orientation}");
        }
        assert_eq!(result.content_type, "image/jpeg");
    }
}

#[test]
fn all_eight_orientations_transform_distinct_corners() {
    let fixture = corner_fixture();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let expected: [[[u8; 3]; 4]; 8] = [
        [[220, 20, 20], [20, 210, 20], [20, 30, 220], [220, 210, 20]],
        [[20, 210, 20], [220, 20, 20], [220, 210, 20], [20, 30, 220]],
        [[220, 210, 20], [20, 30, 220], [20, 210, 20], [220, 20, 20]],
        [[20, 30, 220], [220, 210, 20], [220, 20, 20], [20, 210, 20]],
        [[220, 20, 20], [20, 30, 220], [20, 210, 20], [220, 210, 20]],
        [[20, 30, 220], [220, 20, 20], [220, 210, 20], [20, 210, 20]],
        [[220, 210, 20], [20, 210, 20], [20, 30, 220], [220, 20, 20]],
        [[20, 210, 20], [220, 210, 20], [220, 20, 20], [20, 30, 220]],
    ];
    for orientation in 1..=8 {
        let result = generator
            .generate(
                fixture.path(),
                &spec(DerivativeKind::WallThumbnail, 1024, orientation),
            )
            .unwrap();
        let decoded = image::open(cache.path().join(result.relative_path))
            .unwrap()
            .to_rgb8();
        let (w, h) = decoded.dimensions();
        let points = [
            (w / 8, h / 8),
            (w * 7 / 8, h / 8),
            (w / 8, h * 7 / 8),
            (w * 7 / 8, h * 7 / 8),
        ];
        for (index, (x, y)) in points.into_iter().enumerate() {
            let pixel = decoded.get_pixel(x, y);
            for channel in 0..3 {
                assert!(
                    (i16::from(pixel[channel])
                        - i16::from(expected[orientation as usize - 1][index][channel]))
                    .abs()
                        <= 45,
                    "orientation {orientation}, corner {index}, pixel {:?}",
                    pixel.0
                );
            }
        }
    }
}

#[test]
fn canonical_validation_rejects_every_noncanonical_spec_before_writing() {
    let fixture = fixture();
    let invalid = [
        DerivativeSpec {
            target: DerivativeTarget::LongEdge(1023),
            ..spec(DerivativeKind::WallThumbnail, 1024, 1)
        },
        DerivativeSpec {
            target: DerivativeTarget::LongEdge(1024),
            ..spec(DerivativeKind::ScreenPreview, 4096, 1)
        },
        DerivativeSpec {
            decoder_version: "other".into(),
            ..spec(DerivativeKind::WallThumbnail, 1024, 1)
        },
        DerivativeSpec {
            colour_space: "display-p3".into(),
            ..spec(DerivativeKind::WallThumbnail, 1024, 1)
        },
        DerivativeSpec {
            orientation: 0,
            ..spec(DerivativeKind::WallThumbnail, 1024, 1)
        },
        DerivativeSpec {
            orientation: 9,
            ..spec(DerivativeKind::WallThumbnail, 1024, 1)
        },
        DerivativeSpec {
            kind: DerivativeKind::RawDecode,
            ..spec(DerivativeKind::WallThumbnail, 1024, 1)
        },
        DerivativeSpec {
            kind: DerivativeKind::ScreenPreview,
            target: DerivativeTarget::LongEdge(1024),
            ..spec(DerivativeKind::ScreenPreview, 4096, 1)
        },
    ];
    for spec in invalid {
        let cache = tempfile::tempdir().unwrap();
        let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
        assert!(matches!(
            generator.generate(fixture.path(), &spec),
            Err(ImageDerivativeError::InvalidSpecification)
        ));
        assert!(cache.path().join("00").read_dir().is_err());
    }
}

#[test]
fn managed_screen_budget_authorizes_exact_encoded_bytes_and_reuses_row() {
    let fixture = fixture();
    let cache = tempfile::tempdir().unwrap();
    let library = NewLibrary::configured("Photos", cache.path());
    let asset = photo_catalog::NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(std::path::Path::new("image.jpg")).unwrap(),
        "image.jpg",
        MediaKind::Jpeg,
        1,
    );
    let group = FolderGroupId::new();
    let mut catalog = Catalog::open_in_memory().unwrap();
    catalog.add_library(&library).unwrap();
    catalog.upsert_asset(&asset).unwrap();
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: group,
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(std::path::Path::new("album"))
                .unwrap(),
            display_path: "album".into(),
            last_viewed_at: Some(1),
        })
        .unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let signature = FileSignature {
        size_bytes: std::fs::metadata(fixture.path()).unwrap().len(),
        modified_unix_ns: 1,
        sidecar_modified_unix_ns: None,
    };
    let before = std::fs::read(fixture.path()).unwrap();
    let before_mtime = std::fs::metadata(fixture.path())
        .unwrap()
        .modified()
        .unwrap();
    let protected = ProtectedGroups::default();
    let tight = CacheBudget::from_total_space(0);
    let files_before = walkdir::WalkDir::new(cache.path()).into_iter().count();
    assert!(matches!(
        generator.generate_screen_preview(
            fixture.path(),
            asset.id,
            signature,
            1,
            group,
            &mut catalog,
            tight,
            &protected
        ),
        Err(photo_cache::ImageDerivativeError::Cache(
            photo_cache::CacheError::BudgetExceeded
        ))
    ));
    assert_eq!(catalog.all_derivatives().unwrap().len(), 0);
    assert_eq!(
        walkdir::WalkDir::new(cache.path()).into_iter().count(),
        files_before
    );
    assert_eq!(std::fs::read(fixture.path()).unwrap(), before);
    assert_eq!(
        std::fs::metadata(fixture.path())
            .unwrap()
            .modified()
            .unwrap(),
        before_mtime
    );
    let sufficient = CacheBudget::from_total_space(10_000_000);
    let first = generator
        .generate_screen_preview(
            fixture.path(),
            asset.id,
            signature,
            1,
            group,
            &mut catalog,
            sufficient,
            &protected,
        )
        .unwrap();
    let second = generator
        .generate_screen_preview(
            fixture.path(),
            asset.id,
            signature,
            1,
            group,
            &mut catalog,
            sufficient,
            &protected,
        )
        .unwrap();
    assert!(first.size_bytes > signature.size_bytes);
    assert!(first.size_bytes <= 4096 * 4096 * 3);
    assert_eq!(
        image::image_dimensions(cache.path().join(&first.relative_path)).unwrap(),
        (1200, 800)
    );
    assert_eq!(first.key, second.key);
    assert!(second.reused);
    assert_eq!(catalog.all_derivatives().unwrap().len(), 1);
}
