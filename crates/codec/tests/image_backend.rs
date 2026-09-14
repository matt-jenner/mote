use std::path::Path;

use image::{DynamicImage, GenericImageView, ImageReader, Rgba, RgbaImage};
use photo_codec::{CodecError, MediaKind, decode_display_image, decoder_fingerprint};

fn asymmetric_image() -> RgbaImage {
    RgbaImage::from_fn(5, 3, |x, y| {
        Rgba([
            (x * 37 + y * 11) as u8,
            (x * 13 + y * 53) as u8,
            (x * 71 + y * 19) as u8,
            255,
        ])
    })
}

fn write_fixture(path: &Path, format: image::ImageFormat) {
    DynamicImage::ImageRgba8(asymmetric_image())
        .save_with_format(path, format)
        .unwrap();
}

fn legacy_decode(path: &Path) -> DynamicImage {
    ImageReader::open(path)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
}

#[test]
fn decodes_existing_formats_identically_to_the_image_backend() {
    let directory = tempfile::tempdir().unwrap();
    let fixtures = [
        ("fixture.jpg", image::ImageFormat::Jpeg, MediaKind::Jpeg),
        ("fixture.png", image::ImageFormat::Png, MediaKind::Png),
        ("fixture.tiff", image::ImageFormat::Tiff, MediaKind::Tiff),
        ("fixture.webp", image::ImageFormat::WebP, MediaKind::Webp),
    ];

    for (name, format, kind) in fixtures {
        let path = directory.path().join(name);
        write_fixture(&path, format);

        let expected = legacy_decode(&path);
        let actual = decode_display_image(&path, kind, 1).unwrap();
        assert_eq!(actual.dimensions(), expected.dimensions(), "{name}");
        assert_eq!(
            actual.to_rgba8().as_raw(),
            expected.to_rgba8().as_raw(),
            "{name}"
        );
    }
}

#[test]
fn applies_each_exif_orientation_to_distinct_pixels() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("orientation.png");
    let image = RgbaImage::from_raw(
        3,
        2,
        vec![
            1, 0, 0, 255, 2, 0, 0, 255, 3, 0, 0, 255, 4, 0, 0, 255, 5, 0, 0, 255, 6, 0, 0, 255,
        ],
    )
    .unwrap();
    DynamicImage::ImageRgba8(image)
        .save_with_format(&path, image::ImageFormat::Png)
        .unwrap();

    let expected = [
        (3, 2, vec![1, 2, 3, 4, 5, 6]),
        (3, 2, vec![3, 2, 1, 6, 5, 4]),
        (3, 2, vec![6, 5, 4, 3, 2, 1]),
        (3, 2, vec![4, 5, 6, 1, 2, 3]),
        (2, 3, vec![1, 4, 2, 5, 3, 6]),
        (2, 3, vec![4, 1, 5, 2, 6, 3]),
        (2, 3, vec![6, 3, 5, 2, 4, 1]),
        (2, 3, vec![3, 6, 2, 5, 1, 4]),
    ];

    for (orientation, (width, height, pixels)) in (1_u16..=8).zip(expected) {
        let decoded = decode_display_image(&path, MediaKind::Png, orientation)
            .unwrap()
            .to_rgba8();
        assert_eq!(
            decoded.dimensions(),
            (width, height),
            "orientation {orientation}"
        );
        assert_eq!(
            decoded.pixels().map(|pixel| pixel[0]).collect::<Vec<_>>(),
            pixels,
            "orientation {orientation}"
        );
    }
}

#[test]
fn identifies_the_image_decoder_and_rejects_video() {
    assert_eq!(
        decoder_fingerprint(MediaKind::Jpeg).unwrap(),
        "image-0.25-v1"
    );
    assert_eq!(
        decoder_fingerprint(MediaKind::Webp).unwrap(),
        "image-0.25-v1"
    );
    assert!(matches!(
        decoder_fingerprint(MediaKind::Video),
        Err(CodecError::Unsupported { .. })
    ));
}
