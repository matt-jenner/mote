#![cfg(feature = "heic")]

use image::DynamicImage;
use photo_codec::{
    CodecError, CodecLimit, MediaKind, decode_display_image, decoder_fingerprint, display_shape,
    embedded_exif_tiff,
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
fn embedded_exif_comes_from_the_declared_primary_and_no_exif_is_none() {
    let portrait = fixture("portrait-rotated.heic");
    let before_bytes = fs::read(&portrait).unwrap();
    let before_metadata = fs::metadata(&portrait).unwrap();
    let tiff = embedded_exif_tiff(&portrait, MediaKind::Heif)
        .unwrap()
        .expect("portrait fixture should carry EXIF");
    assert_eq!(&tiff[..4], b"II\x2a\0");
    assert_eq!(fs::read(&portrait).unwrap(), before_bytes);
    let after_metadata = fs::metadata(&portrait).unwrap();
    assert_eq!(
        before_metadata.modified().unwrap(),
        after_metadata.modified().unwrap()
    );
    assert_eq!(before_metadata.permissions(), after_metadata.permissions());

    assert_eq!(
        embedded_exif_tiff(&fixture("iphone-8bit.heic"), MediaKind::Heif).unwrap(),
        None
    );
}

#[test]
fn big_endian_exif_header_is_returned_from_the_primary() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("big-endian.heic");
    let mut bytes = fs::read(fixture("portrait-rotated.heic")).unwrap();
    let header = bytes
        .windows(4)
        .position(|bytes| bytes == b"II\x2a\0")
        .unwrap();
    bytes[header..header + 4].copy_from_slice(b"MM\0\x2a");
    fs::write(&path, bytes).unwrap();

    let tiff = embedded_exif_tiff(&path, MediaKind::Heif).unwrap().unwrap();
    assert_eq!(&tiff[..4], b"MM\0\x2a");
}

#[test]
fn malformed_exif_blocks_return_owned_typed_errors() {
    let original = fs::read(fixture("portrait-rotated.heic")).unwrap();
    let directory = tempfile::tempdir().unwrap();
    for (name, mutate) in [("short", 0u8), ("overflow", 1), ("outside", 2)] {
        let mut bytes = original.clone();
        let header = bytes
            .windows(4)
            .position(|bytes| bytes == b"II\x2a\0")
            .unwrap();
        if mutate == 0 {
            let entry_prefix = [0, 3, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 8];
            let entry = bytes
                .windows(entry_prefix.len())
                .position(|window| window == entry_prefix)
                .unwrap();
            bytes[entry + 16..entry + 20].copy_from_slice(&3u32.to_be_bytes());
        } else if mutate == 1 {
            bytes[header - 4..header].copy_from_slice(&u32::MAX.to_be_bytes());
        } else {
            bytes[header - 4..header].copy_from_slice(&80u32.to_be_bytes());
        }
        let path = directory.path().join(format!("{name}.heic"));
        fs::write(&path, bytes).unwrap();
        assert!(matches!(
            embedded_exif_tiff(&path, MediaKind::Heif),
            Err(CodecError::InvalidExif { .. })
        ));
    }
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
                bytes[i + 6] = 16; // 17 x 241 = 4,097 tiles.
                bytes[i + 7] = 240;
                // Supply the complete reference table so rejection cannot be
                // satisfied by Missing_grid_images. HEVC grid references are
                // bounded by the stricter 1,024-item security limit, before
                // the 4,096 tili limit could apply.
                let dimg = bytes.windows(4).position(|b| b == b"dimg").unwrap();
                let old_size =
                    u32::from_be_bytes(bytes[dimg - 4..dimg].try_into().unwrap()) as usize;
                let mut replacement = Vec::new();
                replacement.extend_from_slice(&(12u32 + 4097 * 2).to_be_bytes());
                replacement.extend_from_slice(b"dimg");
                replacement.extend_from_slice(&2u16.to_be_bytes());
                replacement.extend_from_slice(&4097u16.to_be_bytes());
                for id in 1u16..=4097 {
                    replacement.extend_from_slice(&id.to_be_bytes());
                }
                let delta = replacement.len() - old_size;
                for parent in [b"meta", b"iref"] {
                    let p = bytes.windows(4).position(|b| b == parent).unwrap();
                    let size = u32::from_be_bytes(bytes[p - 4..p].try_into().unwrap());
                    bytes[p - 4..p].copy_from_slice(&(size + delta as u32).to_be_bytes());
                }
                let iloc = bytes.windows(4).position(|b| b == b"iloc").unwrap();
                for base in [iloc + 18, iloc + 58] {
                    let offset = u32::from_be_bytes(bytes[base..base + 4].try_into().unwrap());
                    bytes[base..base + 4].copy_from_slice(&(offset + delta as u32).to_be_bytes());
                }
                bytes.splice(dimg - 4..dimg - 4 + old_size, replacement);
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
        let diagnostic = error.to_string().to_lowercase();
        if kind == "pixels" {
            assert!(matches!(
                error,
                CodecError::LimitExceeded {
                    limit: CodecLimit::DecodedPixels,
                    actual: 400_000_000,
                    maximum: 150_000_000
                }
            ));
        } else {
            assert!(diagnostic.contains("security limit"), "{kind}: {error}");
        }
        if kind == "items" {
            assert!(
                diagnostic.contains("iinf box contains 1025") && diagnostic.contains("1024 items"),
                "{error}"
            );
        }
        if kind == "tiles" {
            assert!(
                diagnostic.contains("references in iref box (4097)")
                    && diagnostic.contains("1024 references"),
                "{error}"
            );
        }
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
    for (position, expected) in [((0, 0), [6u8, 1, 245]), ((92, 99), [252, 254, 3])] {
        for (actual, expected) in image
            .get_pixel(position.0, position.1)
            .0
            .into_iter()
            .zip(expected)
        {
            assert!(actual.abs_diff(expected) <= 2);
        }
    }
}

fn tagged_hdr_fixture(transfer: u16) -> Vec<u8> {
    let mut bytes = fs::read(fixture("iphone-10bit-grid.heic")).unwrap();
    let ipco = bytes.windows(4).position(|b| b == b"ipco").unwrap();
    let size = u32::from_be_bytes(bytes[ipco - 4..ipco].try_into().unwrap()) as usize;
    let mut property = 19u32.to_be_bytes().to_vec();
    property.extend_from_slice(b"colrnclx");
    property.extend_from_slice(&9u16.to_be_bytes()); // BT.2020
    property.extend_from_slice(&transfer.to_be_bytes());
    property.extend_from_slice(&6u16.to_be_bytes());
    property.push(128);
    bytes.splice(ipco - 4 + size..ipco - 4 + size, property);
    bytes[ipco - 4..ipco].copy_from_slice(&(size as u32 + 19).to_be_bytes());
    let ipma = bytes.windows(4).position(|b| b == b"ipma").unwrap();
    let mut entry = ipma + 12;
    while u16::from_be_bytes(bytes[entry..entry + 2].try_into().unwrap()) != 2 {
        entry += 3 + usize::from(bytes[entry + 2]);
    }
    let end = entry + 3 + usize::from(bytes[entry + 2]);
    bytes[entry + 2] += 1;
    bytes.insert(end, 0x85); // Essential property #5 on declared primary.
    for (name, delta) in [(b"ipma", 1u32), (b"iprp", 20), (b"meta", 20)] {
        let p = bytes.windows(4).position(|b| b == name).unwrap();
        let size = u32::from_be_bytes(bytes[p - 4..p].try_into().unwrap());
        bytes[p - 4..p].copy_from_slice(&(size + delta).to_be_bytes());
    }
    let iloc = bytes.windows(4).position(|b| b == b"iloc").unwrap();
    for base in [iloc + 18, iloc + 58] {
        let value = u32::from_be_bytes(bytes[base..base + 4].try_into().unwrap());
        bytes[base..base + 4].copy_from_slice(&(value + 20).to_be_bytes());
    }
    bytes
}

#[test]
fn pq_and_hlg_ten_bit_files_render_reference_neutral_and_saturated_highlight() {
    let directory = tempfile::tempdir().unwrap();
    // Independent ST 2084 / BT.2100 + BT.2020-to-sRGB matrix calculations
    // from native 10-bit samples [252,261,784] and [521,516,487].
    for (transfer, expected) in [
        (16, [[205u8, 209, 255], [233, 223, 193]]),
        (18, [[83, 105, 255], [194, 190, 180]]),
    ] {
        let bytes = tagged_hdr_fixture(transfer);
        let path = directory.path().join("hdr.heic");
        fs::write(&path, bytes).unwrap();
        let image = decode_display_image(&path, MediaKind::Heif, 1).unwrap();
        let DynamicImage::ImageRgb8(image) = image else {
            panic!("expected SDR RGB8")
        };
        assert_eq!(image.dimensions(), (93, 100));
        for ((x, y), expected) in [(7, 25), (14, 50)].into_iter().zip(expected) {
            let pixel = image.get_pixel(x, y).0;
            for (actual, expected) in pixel.into_iter().zip(expected) {
                assert!(
                    actual.abs_diff(expected) <= 2,
                    "transfer {transfer}, {x},{y}: {pixel:?}"
                );
            }
        }
    }
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
