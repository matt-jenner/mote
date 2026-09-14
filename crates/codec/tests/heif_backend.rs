#![cfg(feature = "heic")]

use image::DynamicImage;
use photo_codec::{
    CodecError, CodecLimit, MediaKind, decode_display_image, decoder_fingerprint, display_shape,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/heif")
        .join(name)
}

#[test]
fn manifest_pixels_and_transform_positions_match_display_output() {
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture("fixtures.json")).unwrap()).unwrap();
    for item in manifest["fixtures"].as_array().unwrap() {
        if item["expected_error"].is_string() {
            continue;
        }
        let name = item["file"].as_str().unwrap();
        let image = decode_display_image(&fixture(name), MediaKind::Heif, 8).unwrap();
        let DynamicImage::ImageRgb8(image) = image else {
            panic!("{name}: expected RGB8")
        };
        let (w, h) = (
            item["width"].as_u64().unwrap() as u32,
            item["height"].as_u64().unwrap() as u32,
        );
        assert_eq!(image.dimensions(), (w, h), "{name}");
        for ((x, y), expected) in [(0, 0), (w - 1, 0), (0, h - 1), (w - 1, h - 1)]
            .into_iter()
            .zip(item["corners_srgb"].as_array().unwrap())
        {
            for (actual, expected) in image
                .get_pixel(x, y)
                .0
                .into_iter()
                .zip(expected.as_array().unwrap())
            {
                assert!(
                    actual.abs_diff(expected.as_u64().unwrap() as u8)
                        <= item["tolerance"].as_u64().unwrap() as u8,
                    "{name} corner {x},{y}"
                );
            }
        }
    }
}

#[test]
fn rejects_declared_pixel_item_and_tile_bombs() {
    let original = fs::read(fixture("iphone-10bit-grid.heic")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    for kind in ["pixels", "items", "tiles"] {
        let mut bytes = original.clone();
        match kind {
            "pixels" => {
                let offsets: Vec<_> = bytes
                    .windows(4)
                    .enumerate()
                    .filter_map(|(i, b)| (b == b"ispe").then_some(i))
                    .collect();
                for i in offsets {
                    bytes[i + 8..i + 12].copy_from_slice(&20_000u32.to_be_bytes());
                    bytes[i + 12..i + 16].copy_from_slice(&20_000u32.to_be_bytes());
                }
            }
            "items" => {
                let i = bytes.windows(4).position(|b| b == b"iinf").unwrap();
                bytes[i + 8..i + 10].copy_from_slice(&1025u16.to_be_bytes());
            }
            _ => {
                let i = bytes.windows(4).position(|b| b == b"idat").unwrap();
                bytes[i + 6] = 255;
                bytes[i + 7] = 255;
            }
        }
        let path = directory.path().join(format!("{kind}.heic"));
        fs::write(&path, bytes).unwrap();
        let error = decode_display_image(&path, MediaKind::Heif, 1).unwrap_err();
        assert!(
            matches!(
                error,
                CodecError::LimitExceeded { .. } | CodecError::Container { .. }
            ),
            "{kind}: {error}"
        );
    }
}

#[test]
fn primary_still_decodes_rgb8_and_has_stable_fingerprint() {
    let image = decode_display_image(&fixture("iphone-8bit.heic"), MediaKind::Heif, 1).unwrap();
    let DynamicImage::ImageRgb8(image) = image else {
        panic!("expected SDR RGB8")
    };
    assert_eq!(image.dimensions(), (29, 100));
    assert_eq!(image.get_pixel(0, 0).0, [0, 6, 252]);
    assert_eq!(image.get_pixel(28, 99).0, [252, 251, 1]);
    let shape = display_shape(&fixture("iphone-8bit.heic"), MediaKind::Heif).unwrap();
    assert_eq!((shape.width, shape.height), image.dimensions());
    assert_eq!(
        decoder_fingerprint(MediaKind::Heif).unwrap(),
        "libheif-1.23.4-libde265-1.1.1-sdr-v1"
    );
}

#[test]
fn ten_bit_two_tile_grid_selects_primary_and_reduces_to_rgb8() {
    let image =
        decode_display_image(&fixture("iphone-10bit-grid.heic"), MediaKind::Heif, 1).unwrap();
    let DynamicImage::ImageRgb8(image) = image else {
        panic!("expected SDR RGB8")
    };
    assert_eq!(image.dimensions(), (93, 100));
    assert_eq!(image.get_pixel(0, 0).0, [6, 1, 245]);
    assert_eq!(image.get_pixel(92, 99).0, [252, 254, 3]);
}

#[test]
fn declared_primary_wins_over_an_earlier_top_level_image() {
    let mut bytes = fs::read(fixture("iphone-8bit.heic")).unwrap();
    let infe = bytes.windows(4).position(|b| b == b"infe").unwrap();
    bytes[infe + 7] = 0; // Expose the 64x100 tile as an alternate top-level image.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("collection.heic");
    fs::write(&path, bytes).unwrap();
    let image = decode_display_image(&path, MediaKind::Heif, 1).unwrap();
    assert_eq!((image.width(), image.height()), (29, 100));
    assert_eq!(image.to_rgb8().get_pixel(28, 99).0, [252, 251, 1]);
}

#[test]
fn display_p3_is_converted_to_srgb_rather_than_relabelled() {
    let image = decode_display_image(&fixture("display-p3.heic"), MediaKind::Heif, 1)
        .unwrap()
        .to_rgb8();
    // Independent P3 -> XYZ D65 -> sRGB matrix calculation on [66,65,195].
    let pixel = image.get_pixel(7, 25).0;
    for (actual, expected) in pixel.into_iter().zip([66u8, 65, 203]) {
        assert!(actual.abs_diff(expected) <= 2, "{pixel:?}");
    }
}

#[test]
fn embedded_icc_display_p3_is_converted_and_invalid_icc_is_rejected() {
    let directory = tempfile::tempdir().unwrap();
    for valid in [true, false] {
        let profile = if valid {
            moxcms::ColorProfile::new_display_p3().encode().unwrap()
        } else {
            vec![0; 128]
        };
        let mut bytes = fs::read(fixture("display-p3.heic")).unwrap();
        let position = bytes.windows(4).position(|b| b == b"colr").unwrap();
        let old_size =
            u32::from_be_bytes(bytes[position - 4..position].try_into().unwrap()) as usize;
        let new_size = 12 + profile.len();
        let delta = new_size as i64 - old_size as i64;
        for name in [b"meta", b"iprp", b"ipco"] {
            let p = bytes.windows(4).position(|b| b == name).unwrap();
            let size = u32::from_be_bytes(bytes[p - 4..p].try_into().unwrap());
            bytes[p - 4..p].copy_from_slice(&((i64::from(size) + delta) as u32).to_be_bytes());
        }
        let iloc = bytes.windows(4).position(|b| b == b"iloc").unwrap();
        let offset = u32::from_be_bytes(bytes[iloc + 18..iloc + 22].try_into().unwrap());
        bytes[iloc + 18..iloc + 22]
            .copy_from_slice(&((i64::from(offset) + delta) as u32).to_be_bytes());
        let replacement = [
            (new_size as u32).to_be_bytes().as_slice(),
            b"colrprof",
            &profile,
        ]
        .concat();
        bytes.splice(position - 4..position - 4 + old_size, replacement);
        let path = directory.path().join("icc.heic");
        fs::write(&path, bytes).unwrap();
        let result = decode_display_image(&path, MediaKind::Heif, 1);
        if valid {
            let pixel = result.unwrap().to_rgb8().get_pixel(7, 25).0;
            for (actual, expected) in pixel.into_iter().zip([66u8, 65, 203]) {
                assert!(actual.abs_diff(expected) <= 2, "{pixel:?}");
            }
        } else {
            assert!(matches!(result, Err(CodecError::Decode { .. })));
        }
    }
}

#[test]
fn rejects_truncation_before_decode() {
    assert!(matches!(
        decode_display_image(&fixture("truncated.heic"), MediaKind::Heif, 1),
        Err(CodecError::Container { .. })
    ));
}

#[test]
fn rejects_sparse_oversized_input_before_reading_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("large.heic");
    fs::File::create(&path)
        .unwrap()
        .set_len(512 * 1024 * 1024 + 1)
        .unwrap();
    assert!(matches!(
        decode_display_image(&path, MediaKind::Heif, 1),
        Err(CodecError::LimitExceeded {
            limit: CodecLimit::EncodedBytes,
            ..
        })
    ));
}

// macOS/APFS rejects invalid UTF-8 names before the decoder is reached.
// Linux CI exercises this on Unix filesystems accepting arbitrary byte names.
#[cfg(all(unix, not(target_os = "macos")))]
#[test]
fn non_utf8_source_path_is_supported_without_source_changes() {
    use std::os::unix::ffi::OsStringExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory
        .path()
        .join(std::ffi::OsString::from_vec(b"photo-\xff.heic".to_vec()));
    assert_source_unchanged(&path);
}

#[test]
fn source_bytes_modified_time_and_permissions_are_unchanged() {
    let directory = tempfile::tempdir().unwrap();
    assert_source_unchanged(&directory.path().join("photo.heic"));
}

fn assert_source_unchanged(path: &Path) {
    let bytes = fs::read(fixture("iphone-8bit.heic")).unwrap();
    fs::write(path, &bytes).unwrap();
    let before = fs::metadata(path).unwrap();
    let decoded = decode_display_image(path, MediaKind::Heif, 6).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (29, 100));
    assert_eq!(fs::read(path).unwrap(), bytes);
    let after = fs::metadata(path).unwrap();
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
    assert_eq!(before.permissions(), after.permissions());
}
