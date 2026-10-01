#[cfg(feature = "heic")]
use image::GenericImageView;
use photo_cache::{
    CacheBudget, DerivativeKind, DerivativeSpec, DerivativeTarget, ImageDerivativeError,
    ImageDerivativeGenerator, ProtectedGroups,
};
use photo_catalog::{Catalog, NewFolderGroup, NewLibrary};
use photo_domain::{AssetId, FileSignature, FolderGroupId, LibraryId, MediaKind, RelativePathKey};
#[cfg(feature = "heic")]
use sha2::{Digest, Sha256};

fn spec(kind: DerivativeKind, edge: u32, orientation: u16) -> DerivativeSpec {
    spec_for_media(kind, edge, orientation, MediaKind::Jpeg)
}

fn spec_for_media(
    kind: DerivativeKind,
    edge: u32,
    orientation: u16,
    media_kind: MediaKind,
) -> DerivativeSpec {
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
        media_kind,
        orientation,
        kind,
        decoder_version: photo_codec::decoder_fingerprint(media_kind)
            .unwrap()
            .to_owned(),
        colour_space: "srgb".into(),
        target: DerivativeTarget::LongEdge(edge),
    }
}

#[cfg(feature = "heic")]
fn heif_fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../codec/tests/fixtures/heif")
        .join(name)
}

#[cfg(feature = "heic")]
fn sha256_hex(bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(bytes);
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(feature = "heic")]
#[derive(Clone)]
struct IsoBox {
    kind: [u8; 4],
    data: Vec<u8>,
}

#[cfg(feature = "heic")]
fn parse_iso_boxes(bytes: &[u8]) -> Vec<IsoBox> {
    let mut boxes = Vec::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        assert!(offset + 8 <= bytes.len(), "truncated ISO-BMFF box header");
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        assert!(
            size >= 8 && offset + size <= bytes.len(),
            "invalid ISO-BMFF box size"
        );
        boxes.push(IsoBox {
            kind: bytes[offset + 4..offset + 8].try_into().unwrap(),
            data: bytes[offset + 8..offset + size].to_vec(),
        });
        offset += size;
    }
    boxes
}

#[cfg(feature = "heic")]
fn serialize_iso_boxes(boxes: &[IsoBox]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for item in boxes {
        let size = u32::try_from(item.data.len() + 8).unwrap();
        bytes.extend_from_slice(&size.to_be_bytes());
        bytes.extend_from_slice(&item.kind);
        bytes.extend_from_slice(&item.data);
    }
    bytes
}

#[cfg(feature = "heic")]
fn iso_box_mut<'a>(boxes: &'a mut [IsoBox], kind: &[u8; 4]) -> &'a mut IsoBox {
    boxes
        .iter_mut()
        .find(|item| &item.kind == kind)
        .unwrap_or_else(|| panic!("missing ISO-BMFF box {}", String::from_utf8_lossy(kind)))
}

/// Expands the licensed 64x100 encoded tile through container metadata only.
/// Sixty-five distinct item IDs share the same encoded extent, producing a
/// compact 4160x100 grid without invoking or depending on an encoder.
#[cfg(feature = "heic")]
fn oversized_heif_grid() -> tempfile::NamedTempFile {
    const COLUMNS: u16 = 65;
    const WIDTH: u16 = 4160;
    const HEIGHT: u16 = 100;

    let original = std::fs::read(heif_fixture("iphone-8bit.heic")).unwrap();
    let mut top = parse_iso_boxes(&original);
    let meta = iso_box_mut(&mut top, b"meta");
    let original_meta_len = meta.data.len();
    let mut boxes = parse_iso_boxes(&meta.data[4..]);

    let grid = &mut iso_box_mut(&mut boxes, b"idat").data;
    assert_eq!(grid.len(), 8);
    grid[2] = 0;
    grid[3] = u8::try_from(COLUMNS - 1).unwrap();
    grid[4..6].copy_from_slice(&WIDTH.to_be_bytes());
    grid[6..8].copy_from_slice(&HEIGHT.to_be_bytes());

    let iref = iso_box_mut(&mut boxes, b"iref");
    let mut references = parse_iso_boxes(&iref.data[4..]);
    let dimg = iso_box_mut(&mut references, b"dimg");
    dimg.data.clear();
    dimg.data.extend_from_slice(&2u16.to_be_bytes());
    dimg.data.extend_from_slice(&COLUMNS.to_be_bytes());
    dimg.data.extend_from_slice(&1u16.to_be_bytes());
    for id in 3..=COLUMNS + 1 {
        dimg.data.extend_from_slice(&id.to_be_bytes());
    }
    iref.data.truncate(4);
    iref.data
        .extend_from_slice(&serialize_iso_boxes(&references));

    let iloc = iso_box_mut(&mut boxes, b"iloc");
    assert_eq!(iloc.data.len(), 48);
    let tile_location = iloc.data[8..28].to_vec();
    iloc.data[6..8].copy_from_slice(&(COLUMNS + 1).to_be_bytes());
    for id in 3..=COLUMNS + 1 {
        let mut location = tile_location.clone();
        location[0..2].copy_from_slice(&id.to_be_bytes());
        iloc.data.extend_from_slice(&location);
    }

    let iinf = iso_box_mut(&mut boxes, b"iinf");
    let mut entries = parse_iso_boxes(&iinf.data[6..]);
    let tile_info = entries[0].clone();
    for id in 3..=COLUMNS + 1 {
        let mut info = tile_info.clone();
        info.data[4..6].copy_from_slice(&id.to_be_bytes());
        entries.push(info);
    }
    iinf.data.truncate(6);
    iinf.data[4..6].copy_from_slice(&(COLUMNS + 1).to_be_bytes());
    iinf.data.extend_from_slice(&serialize_iso_boxes(&entries));

    let iprp = iso_box_mut(&mut boxes, b"iprp");
    let mut property_boxes = parse_iso_boxes(&iprp.data);
    let ipco = iso_box_mut(&mut property_boxes, b"ipco");
    let mut properties = parse_iso_boxes(&ipco.data);
    assert_eq!(&properties[2].kind, b"ispe");
    properties[2].data[4..8].copy_from_slice(&u32::from(WIDTH).to_be_bytes());
    properties[2].data[8..12].copy_from_slice(&u32::from(HEIGHT).to_be_bytes());
    ipco.data = serialize_iso_boxes(&properties);

    let ipma = iso_box_mut(&mut property_boxes, b"ipma");
    assert_eq!(ipma.data.len(), 18);
    let tile_properties = ipma.data[8..13].to_vec();
    ipma.data[4..8].copy_from_slice(&u32::from(COLUMNS + 1).to_be_bytes());
    for id in 3..=COLUMNS + 1 {
        let mut association = tile_properties.clone();
        association[0..2].copy_from_slice(&id.to_be_bytes());
        ipma.data.extend_from_slice(&association);
    }
    iprp.data = serialize_iso_boxes(&property_boxes);

    meta.data = [meta.data[..4].to_vec(), serialize_iso_boxes(&boxes)].concat();
    let delta = u32::try_from(meta.data.len() - original_meta_len).unwrap();
    let iloc = iso_box_mut(&mut boxes, b"iloc");
    for item_index in std::iter::once(0usize).chain(2..usize::from(COLUMNS + 1)) {
        let base_offset = 8 + item_index * 20 + 6;
        let old = u32::from_be_bytes(iloc.data[base_offset..base_offset + 4].try_into().unwrap());
        iloc.data[base_offset..base_offset + 4]
            .copy_from_slice(&old.checked_add(delta).unwrap().to_be_bytes());
    }
    meta.data = [meta.data[..4].to_vec(), serialize_iso_boxes(&boxes)].concat();

    let derived = serialize_iso_boxes(&top);
    assert!(
        derived.len() < 16 * 1024,
        "container-only fixture must remain compact"
    );
    let mut file = tempfile::Builder::new().suffix(".heic").tempfile().unwrap();
    std::io::Write::write_all(file.as_file_mut(), &derived).unwrap();
    file
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
fn repairs_a_wall_thumbnail_from_a_cached_screen_preview() {
    let fixture = fixture();
    let cache = tempfile::tempdir().unwrap();
    let cached_screen = cache.path().join("legacy-screen.jpg");
    std::fs::copy(fixture.path(), &cached_screen).unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();

    let repaired = generator
        .generate_wall_thumbnail_from_cached_preview(
            std::path::Path::new("legacy-screen.jpg"),
            &spec(DerivativeKind::WallThumbnail, 1024, 1),
        )
        .unwrap();

    assert!(repaired.durable);
    assert_eq!(
        image::image_dimensions(cache.path().join(repaired.relative_path)).unwrap(),
        (1024, 683)
    );
}

#[test]
fn cached_preview_repair_replaces_an_orphaned_corrupt_regular_file() {
    let fixture = fixture();
    let cache = tempfile::tempdir().unwrap();
    let cached_screen = cache.path().join("legacy-screen.jpg");
    std::fs::copy(fixture.path(), &cached_screen).unwrap();
    let requested = spec(DerivativeKind::WallThumbnail, 1024, 1);
    let key = photo_cache::DerivativeKey::compute(&requested);
    let relative = key.sharded_path("jpg");
    std::fs::create_dir_all(cache.path().join(relative.parent().unwrap())).unwrap();
    std::fs::write(cache.path().join(&relative), b"orphaned corrupt bytes").unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();

    generator
        .generate_wall_thumbnail_from_cached_preview(
            std::path::Path::new("legacy-screen.jpg"),
            &requested,
        )
        .unwrap();
    assert!(image::load_from_memory(&std::fs::read(cache.path().join(relative)).unwrap()).is_ok());
}

#[test]
fn generate_replaces_an_orphaned_corrupt_regular_file() {
    let fixture = fixture();
    let cache = tempfile::tempdir().unwrap();
    let requested = spec(DerivativeKind::WallThumbnail, 1024, 1);
    let key = photo_cache::DerivativeKey::compute(&requested);
    let relative = key.sharded_path("jpg");
    std::fs::create_dir_all(cache.path().join(relative.parent().unwrap())).unwrap();
    std::fs::write(cache.path().join(&relative), b"orphaned corrupt bytes").unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();

    let generated = generator.generate(fixture.path(), &requested).unwrap();

    assert!(!generated.reused);
    assert!(image::load_from_memory(&std::fs::read(cache.path().join(relative)).unwrap()).is_ok());
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
#[cfg(feature = "heic")]
fn fingerprint_validation_binds_the_decoder_to_the_media_kind() {
    let source = heif_fixture("iphone-8bit.heic");
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let heif_with_jpeg_fingerprint = DerivativeSpec {
        decoder_version: photo_codec::decoder_fingerprint(MediaKind::Jpeg)
            .unwrap()
            .to_owned(),
        ..spec_for_media(DerivativeKind::WallThumbnail, 1024, 1, MediaKind::Heif)
    };
    assert!(matches!(
        generator.generate(&source, &heif_with_jpeg_fingerprint),
        Err(ImageDerivativeError::InvalidSpecification)
    ));

    let jpeg_with_heif_fingerprint = DerivativeSpec {
        decoder_version: photo_codec::decoder_fingerprint(MediaKind::Heif)
            .unwrap()
            .to_owned(),
        ..spec(DerivativeKind::WallThumbnail, 1024, 1)
    };
    assert!(matches!(
        generator.generate(fixture().path(), &jpeg_with_heif_fingerprint),
        Err(ImageDerivativeError::InvalidSpecification)
    ));
}

#[test]
fn declared_media_kind_controls_source_decoding_instead_of_the_extension() {
    let jpeg = fixture();
    let renamed = tempfile::Builder::new().suffix(".heic").tempfile().unwrap();
    std::fs::copy(jpeg.path(), renamed.path()).unwrap();
    let cache = tempfile::tempdir().unwrap();

    let generated = ImageDerivativeGenerator::new(cache.path())
        .unwrap()
        .generate(
            renamed.path(),
            &spec_for_media(DerivativeKind::WallThumbnail, 1024, 1, MediaKind::Jpeg),
        )
        .unwrap();

    assert_eq!(
        image::image_dimensions(cache.path().join(generated.relative_path)).unwrap(),
        (1024, 683)
    );
}

#[test]
#[cfg(feature = "heic")]
fn real_heif_wall_and_screen_derivatives_are_jpegs_reused_and_leave_source_unchanged() {
    let source = heif_fixture("iphone-8bit.heic");
    let source_bytes = std::fs::read(&source).unwrap();
    let source_sha256 = sha256_hex(&source_bytes);
    assert_eq!(
        source_sha256,
        "a28a4106084425aaf4b5b77aba75c397ed4a0d87857c1f349bd0ab7bb35fc884"
    );
    let source_metadata = std::fs::metadata(&source).unwrap();
    let source_modified = source_metadata.modified().unwrap();
    #[cfg(unix)]
    let source_mode = {
        use std::os::unix::fs::PermissionsExt;
        source_metadata.permissions().mode()
    };

    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let wall_spec = spec_for_media(DerivativeKind::WallThumbnail, 1024, 1, MediaKind::Heif);
    let wall = generator.generate(&source, &wall_spec).unwrap();
    let reused_wall = generator.generate(&source, &wall_spec).unwrap();
    let wall_bytes = std::fs::read(cache.path().join(&wall.relative_path)).unwrap();
    assert_eq!(
        image::guess_format(&wall_bytes).unwrap(),
        image::ImageFormat::Jpeg
    );
    assert_eq!(
        image::load_from_memory(&wall_bytes).unwrap().dimensions(),
        (29, 100)
    );
    assert!(reused_wall.reused);

    let library = NewLibrary::configured("Photos", cache.path());
    let mut asset = photo_catalog::NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(std::path::Path::new("iphone-8bit.heic")).unwrap(),
        "iphone-8bit.heic",
        MediaKind::Heif,
        source_metadata.len(),
    );
    let group = FolderGroupId::new();
    asset.folder_group_id = Some(group);
    let mut catalog = Catalog::open_in_memory().unwrap();
    catalog.add_library(&library).unwrap();
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: group,
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(std::path::Path::new("album"))
                .unwrap(),
            display_path: "album".to_owned(),
            last_viewed_at: Some(1),
        })
        .unwrap();
    catalog.upsert_asset(&asset).unwrap();
    let signature = FileSignature {
        size_bytes: source_metadata.len(),
        modified_unix_ns: 1,
        sidecar_modified_unix_ns: None,
    };
    let budget = CacheBudget::from_total_space(10_000_000);
    let protected = ProtectedGroups::default();
    let screen = generator
        .generate_screen_preview(
            &source,
            asset.id,
            signature,
            1,
            MediaKind::Heif,
            group,
            &mut catalog,
            budget,
            &protected,
        )
        .unwrap();
    let reused_screen = generator
        .generate_screen_preview(
            &source,
            asset.id,
            signature,
            1,
            MediaKind::Heif,
            group,
            &mut catalog,
            budget,
            &protected,
        )
        .unwrap();
    let screen_bytes = std::fs::read(cache.path().join(&screen.relative_path)).unwrap();
    assert_eq!(
        image::guess_format(&screen_bytes).unwrap(),
        image::ImageFormat::Jpeg
    );
    assert_eq!(
        image::load_from_memory(&screen_bytes).unwrap().dimensions(),
        (29, 100)
    );
    assert!(reused_screen.reused);

    for representative in [wall.representative_rgb, screen.representative_rgb] {
        for (actual, expected) in [
            (representative.red, 126_u8),
            (representative.green, 128_u8),
            (representative.blue, 126_u8),
        ] {
            assert!(
                actual.abs_diff(expected) <= 6,
                "representative channel {actual}"
            );
        }
    }
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    assert_eq!(sha256_hex(&std::fs::read(&source).unwrap()), source_sha256);
    assert_eq!(
        std::fs::metadata(&source).unwrap().modified().unwrap(),
        source_modified
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&source).unwrap().permissions().mode(),
            source_mode
        );
    }
}

#[test]
#[cfg(feature = "heic")]
fn real_oversized_heif_uses_distinct_wall_and_screen_long_edge_caps() {
    let source = oversized_heif_grid();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();

    let wall = generator
        .generate(
            source.path(),
            &spec_for_media(DerivativeKind::WallThumbnail, 1024, 1, MediaKind::Heif),
        )
        .unwrap();
    let wall_bytes = std::fs::read(cache.path().join(wall.relative_path)).unwrap();
    assert_eq!(
        image::guess_format(&wall_bytes).unwrap(),
        image::ImageFormat::Jpeg
    );
    assert_eq!(
        image::load_from_memory(&wall_bytes).unwrap().dimensions(),
        (1024, 25)
    );

    let screen = generator
        .encode_screen_preview(
            source.path(),
            &spec_for_media(DerivativeKind::ScreenPreview, 4096, 1, MediaKind::Heif),
        )
        .unwrap();
    assert_eq!(
        image::guess_format(&screen.bytes).unwrap(),
        image::ImageFormat::Jpeg
    );
    assert_eq!(
        image::load_from_memory(&screen.bytes).unwrap().dimensions(),
        (4096, 98)
    );
}

#[test]
#[cfg(feature = "heic")]
fn heif_container_orientation_is_not_applied_again() {
    let source = heif_fixture("portrait-rotated.heic");
    let cache = tempfile::tempdir().unwrap();
    let generated = ImageDerivativeGenerator::new(cache.path())
        .unwrap()
        .generate(
            &source,
            &spec_for_media(DerivativeKind::WallThumbnail, 1024, 6, MediaKind::Heif),
        )
        .unwrap();

    assert_eq!(
        image::image_dimensions(cache.path().join(generated.relative_path)).unwrap(),
        (100, 28)
    );
}

#[test]
fn commit_wall_thumbnail_replaces_corrupt_bytes_under_an_immutable_key() {
    let fixture = fixture();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let requested = spec(DerivativeKind::WallThumbnail, 1024, 1);
    let first = generator.generate(fixture.path(), &requested).unwrap();
    std::fs::write(cache.path().join(&first.relative_path), b"corrupt").unwrap();
    let encoded = generator
        .encode_wall_thumbnail(fixture.path(), &requested)
        .unwrap();

    generator
        .commit_wall_thumbnail(encoded, &requested)
        .expect("repair should publish replacement bytes");
    assert!(image::open(cache.path().join(first.relative_path)).is_ok());
}

#[test]
fn valid_png_at_an_immutable_jpg_key_is_replaced_with_jpeg() {
    let fixture = fixture();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let requested = spec(DerivativeKind::WallThumbnail, 1024, 1);
    let first = generator.generate(fixture.path(), &requested).unwrap();
    let cache_path = cache.path().join(&first.relative_path);
    image::ImageBuffer::from_pixel(8, 8, image::Rgb([10_u8, 20_u8, 30_u8]))
        .save_with_format(&cache_path, image::ImageFormat::Png)
        .unwrap();

    let repaired = generator.generate(fixture.path(), &requested).unwrap();

    assert!(!repaired.reused, "a PNG must not be reused as image/jpeg");
    assert_eq!(
        image::guess_format(&std::fs::read(cache_path).unwrap()).unwrap(),
        image::ImageFormat::Jpeg
    );
}

#[test]
fn shared_screen_repair_failure_preserves_bytes_and_existing_group_link() {
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
    let first_group = FolderGroupId::new();
    let second_group = FolderGroupId::new();
    let mut catalog = Catalog::open_in_memory().unwrap();
    catalog.add_library(&library).unwrap();
    catalog.upsert_asset(&asset).unwrap();
    for (group, name) in [(first_group, "first"), (second_group, "second")] {
        catalog
            .upsert_folder_group(&NewFolderGroup {
                id: group,
                library_id: library.id,
                relative_path: RelativePathKey::from_relative_path(std::path::Path::new(name))
                    .unwrap(),
                display_path: name.into(),
                last_viewed_at: Some(1),
            })
            .unwrap();
    }
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let source_signature = FileSignature {
        size_bytes: std::fs::metadata(fixture.path()).unwrap().len(),
        modified_unix_ns: 1,
        sidecar_modified_unix_ns: None,
    };
    let requested = DerivativeSpec {
        asset_id: asset.id,
        signature: source_signature,
        media_kind: MediaKind::Jpeg,
        orientation: 1,
        kind: DerivativeKind::ScreenPreview,
        decoder_version: photo_codec::decoder_fingerprint(MediaKind::Jpeg)
            .unwrap()
            .to_owned(),
        colour_space: "srgb".into(),
        target: DerivativeTarget::LongEdge(4096),
    };
    let budget = CacheBudget::from_total_space(100_000_000);
    let first = generator
        .generate_screen_preview(
            fixture.path(),
            asset.id,
            source_signature,
            1,
            MediaKind::Jpeg,
            first_group,
            &mut catalog,
            budget,
            &ProtectedGroups::default(),
        )
        .unwrap();
    let cache_path = cache.path().join(&first.relative_path);
    std::fs::write(&cache_path, b"old-shared-bytes").unwrap();
    let encoded = generator
        .encode_screen_preview(fixture.path(), &requested)
        .unwrap();
    generator.fail_next_replace_for_test();
    let failed = generator.commit_screen_preview_repairing(
        encoded.clone(),
        &requested,
        second_group,
        &mut catalog,
        budget,
        &ProtectedGroups::default(),
    );
    assert!(matches!(
        failed,
        Err(photo_cache::ImageDerivativeError::Cache(
            photo_cache::CacheError::Write(_)
        ))
    ));
    assert_eq!(std::fs::read(&cache_path).unwrap(), b"old-shared-bytes");
    assert_eq!(catalog.derivative_count(first_group, false).unwrap(), 1);
    assert_eq!(catalog.derivative_count(second_group, false).unwrap(), 0);

    generator
        .commit_screen_preview_repairing(
            encoded,
            &requested,
            second_group,
            &mut catalog,
            budget,
            &ProtectedGroups::default(),
        )
        .unwrap();
    assert_eq!(catalog.derivative_count(first_group, false).unwrap(), 1);
    assert_eq!(catalog.derivative_count(second_group, false).unwrap(), 1);
    assert!(image::open(&cache_path).is_ok());
}

#[test]
fn controlled_demo_fixture_generation_leaves_source_unchanged() {
    let source = fixture();
    let before = std::fs::read(source.path()).unwrap();
    let before_modified = std::fs::metadata(source.path())
        .unwrap()
        .modified()
        .unwrap();
    let cache = tempfile::tempdir().unwrap();
    let generator = ImageDerivativeGenerator::new(cache.path()).unwrap();

    let generated = generator
        .generate(source.path(), &spec(DerivativeKind::WallThumbnail, 1024, 1))
        .unwrap();

    assert!(cache.path().join(generated.relative_path).is_file());
    assert_eq!(std::fs::read(source.path()).unwrap(), before);
    assert_eq!(
        std::fs::metadata(source.path())
            .unwrap()
            .modified()
            .unwrap(),
        before_modified
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
            MediaKind::Jpeg,
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
            MediaKind::Jpeg,
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
            MediaKind::Jpeg,
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

#[test]
fn independent_public_generators_serialize_screen_budget_transactions() {
    let fixture = fixture();
    let source_before = std::fs::read(fixture.path()).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let catalog_path = temp.path().join("catalog.sqlite");
    let library = NewLibrary::configured("Photos", &temp.path().join("photos"));
    let left = photo_catalog::NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(std::path::Path::new("left.jpg")).unwrap(),
        "left.jpg",
        MediaKind::Jpeg,
        1,
    );
    let right = photo_catalog::NewAsset::minimal(
        library.id,
        RelativePathKey::from_relative_path(std::path::Path::new("right.jpg")).unwrap(),
        "right.jpg",
        MediaKind::Jpeg,
        1,
    );
    let group = FolderGroupId::new();
    let mut catalog = Catalog::open(&catalog_path).unwrap();
    catalog.add_library(&library).unwrap();
    catalog.upsert_asset(&left).unwrap();
    catalog.upsert_asset(&right).unwrap();
    catalog
        .upsert_folder_group(&NewFolderGroup {
            id: group,
            library_id: library.id,
            relative_path: RelativePathKey::from_relative_path(std::path::Path::new("album"))
                .unwrap(),
            display_path: "album".to_owned(),
            last_viewed_at: Some(1),
        })
        .unwrap();
    let first_generator = ImageDerivativeGenerator::new(cache.path()).unwrap();
    let encoded_size = first_generator
        .encode_screen_preview(
            fixture.path(),
            &spec_for_asset(left.id, DerivativeKind::ScreenPreview),
        )
        .unwrap()
        .bytes
        .len() as u64;
    let budget = CacheBudget::from_total_space(encoded_size.saturating_mul(10));
    assert_eq!(budget.limit_bytes(), encoded_size);
    let protected = ProtectedGroups::default();
    protected.protect(group).unwrap();
    let signature = FileSignature {
        size_bytes: std::fs::metadata(fixture.path()).unwrap().len(),
        modified_unix_ns: 1,
        sidecar_modified_unix_ns: None,
    };
    let run = |asset_id, generator: ImageDerivativeGenerator| {
        let catalog_path = catalog_path.clone();
        let source = fixture.path().to_owned();
        let protected = protected.clone();
        std::thread::spawn(move || {
            let mut catalog = Catalog::open(&catalog_path).unwrap();
            generator.generate_screen_preview(
                &source,
                asset_id,
                signature,
                1,
                MediaKind::Jpeg,
                group,
                &mut catalog,
                budget,
                &protected,
            )
        })
    };
    let left_thread = run(left.id, first_generator);
    let right_thread = run(
        right.id,
        ImageDerivativeGenerator::new(cache.path()).unwrap(),
    );
    let left_result = left_thread.join().unwrap();
    let right_result = right_thread.join().unwrap();
    let results = [left_result, right_result];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
    assert!(results.iter().any(|result| {
        matches!(
            result,
            Err(ImageDerivativeError::Cache(
                photo_cache::CacheError::BudgetExceeded
            ))
        )
    }));
    assert_eq!(
        Catalog::open(&catalog_path)
            .unwrap()
            .all_derivatives()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(std::fs::read(fixture.path()).unwrap(), source_before);
}

fn spec_for_asset(asset_id: AssetId, kind: DerivativeKind) -> DerivativeSpec {
    DerivativeSpec {
        asset_id,
        signature: FileSignature {
            size_bytes: 1,
            modified_unix_ns: 1,
            sidecar_modified_unix_ns: None,
        },
        media_kind: MediaKind::Jpeg,
        orientation: 1,
        kind,
        decoder_version: photo_codec::decoder_fingerprint(MediaKind::Jpeg)
            .unwrap()
            .to_owned(),
        colour_space: "srgb".into(),
        target: DerivativeTarget::LongEdge(4096),
    }
}
