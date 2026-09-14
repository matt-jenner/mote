use std::{
    fs::File,
    io::{self, Read},
    path::Path,
    sync::OnceLock,
};

use crate::{CodecError, CodecLimit, DisplayShape};
use image::{DynamicImage, RgbImage};
use libheif_rs::{
    ColorSpace, DecodingOptions, HeifContext, ImageHandle, LibHeif, RgbChroma, SecurityLimits,
};
use moxcms::{CicpProfile, ColorProfile, Layout, MatrixCoefficients, ParsingOptions};

const MAX_INPUT: u64 = 512 * 1024 * 1024;
const MAX_PIXELS: u64 = 150_000_000;
const MAX_PROFILE: u32 = 16 * 1024 * 1024;
const MAX_ALLOCATION: u64 = 512 * 1024 * 1024;
const MAX_MEMORY: u64 = 768 * 1024 * 1024;
static LIBRARY: OnceLock<LibHeif> = OnceLock::new();

fn container(error: impl std::fmt::Display) -> CodecError {
    CodecError::Container {
        message: error.to_string(),
    }
}
fn decode_error(error: impl std::fmt::Display) -> CodecError {
    CodecError::Decode {
        message: error.to_string(),
    }
}
fn io_error(path: &Path, error: io::Error) -> CodecError {
    CodecError::Io {
        path: path.into(),
        kind: error.kind(),
        message: error.to_string(),
    }
}
fn check(limit: CodecLimit, actual: u64, maximum: u64) -> Result<(), CodecError> {
    if actual > maximum {
        Err(CodecError::LimitExceeded {
            limit,
            actual,
            maximum,
        })
    } else {
        Ok(())
    }
}

fn source_bytes(path: &Path) -> Result<Vec<u8>, CodecError> {
    let mut file = File::open(path).map_err(|e| io_error(path, e))?;
    let length = file.metadata().map_err(|e| io_error(path, e))?.len();
    check(CodecLimit::EncodedBytes, length, MAX_INPUT)?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(length as usize)
        .map_err(decode_error)?;
    // Bound reads as well as metadata: the file can grow after it was opened.
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut chunk).map_err(|e| io_error(path, e))?;
        if count == 0 {
            break;
        }
        let wanted = bytes.len() as u64 + count as u64;
        check(CodecLimit::EncodedBytes, wanted, MAX_INPUT)?;
        bytes.try_reserve_exact(count).map_err(decode_error)?;
        bytes.extend_from_slice(&chunk[..count]);
    }
    Ok(bytes)
}

fn context(bytes: &[u8]) -> Result<HeifContext<'_>, CodecError> {
    LIBRARY.get_or_init(LibHeif::new);
    let mut context = HeifContext::new().map_err(container)?;
    let mut limits = SecurityLimits::new();
    limits.set_max_image_size_pixels(MAX_PIXELS);
    limits.set_max_items(1_024);
    limits.set_max_number_of_tiles(4_096);
    limits.set_max_color_profile_size(MAX_PROFILE);
    limits.set_max_memory_block_size(MAX_ALLOCATION);
    limits.set_max_total_memory(MAX_MEMORY);
    // Explicit values also prevent inherited/global disabled limits from
    // weakening ancillary container parsing limits.
    limits.set_max_bayer_pattern_pixels(256);
    limits.set_max_components(256);
    limits.set_max_iloc_extents_per_item(4_096);
    limits.set_max_size_entity_group(1_024);
    limits.set_max_children_per_box(4_096);
    limits.set_max_sample_description_box_entries(1_024);
    limits.set_max_sample_group_description_box_entries(1_024);
    limits.set_max_sequence_frames(1_024);
    limits.set_max_number_of_file_brands(64);
    limits.set_max_bad_pixels(1_024);
    limits.set_max_iso23001_17_pixel_size_bytes(256);
    context.set_security_limits(&limits).map_err(container)?;
    context.set_max_decoding_threads(1);
    // read_from_bytes() constructs and parses immediately; the mutable API
    // is required to install production limits BEFORE parsing.
    context.read_bytes(bytes).map_err(container)?;
    Ok(context)
}

fn primary(context: &HeifContext<'_>) -> Result<ImageHandle, CodecError> {
    let handle = context.primary_image_handle().map_err(|error| {
        if matches!(
            error.sub_code,
            libheif_rs::HeifErrorSubCode::NoOrInvalidPrimaryItem
        ) {
            CodecError::MissingPrimaryImage
        } else {
            container(error)
        }
    })?;
    shape(&handle)?;
    Ok(handle)
}

fn shape(handle: &ImageHandle) -> Result<DisplayShape, CodecError> {
    let (width, height) = (handle.width(), handle.height());
    check(
        CodecLimit::DecodedPixels,
        u64::from(width) * u64::from(height),
        MAX_PIXELS,
    )?;
    if width == 0 || height == 0 {
        return Err(container("zero image dimensions"));
    }
    Ok(DisplayShape { width, height })
}

pub(crate) fn display_shape(path: &Path) -> Result<DisplayShape, CodecError> {
    let bytes = source_bytes(path)?;
    let context = context(&bytes)?;
    shape(&primary(&context)?)
}

pub(crate) fn decode(path: &Path) -> Result<DynamicImage, CodecError> {
    let bytes = source_bytes(path)?;
    let context = context(&bytes)?;
    let handle = primary(&context)?;
    let expected = shape(&handle)?;
    let high_precision = handle
        .luma_bits_per_pixel()
        .max(handle.chroma_bits_per_pixel())
        > 8;
    let mut options =
        DecodingOptions::new().ok_or_else(|| decode_error("cannot allocate decoding options"))?;
    options.set_ignore_transformations(false);
    options.set_strict_decoding(true);
    // Retain source precision until HDR luminance mapping. Reducing PQ/HLG
    // code values first destroys shadow detail, even if output is SDR RGB8.
    options.set_convert_hdr_to_8bit(!high_precision);
    // libheif converts YCbCr layout/range and bit depth, but is not an ICC
    // colour-management engine. Preserve source colourimetry for moxcms.
    options.set_output_image_nclx_profile_passthrough(true);
    options
        .set_decoder_id(Some("libde265"))
        .map_err(decode_error)?;
    let image = LIBRARY
        .get_or_init(LibHeif::new)
        .decode(
            &handle,
            ColorSpace::Rgb(if high_precision {
                RgbChroma::HdrRgbLe
            } else {
                RgbChroma::Rgb
            }),
            Some(options),
        )
        .map_err(decode_error)?;
    if image.width() != expected.width || image.height() != expected.height {
        return Err(decode_error(
            "decoded dimensions disagree with primary display dimensions",
        ));
    }
    let plane = image
        .planes()
        .interleaved
        .ok_or(CodecError::MissingInterleavedPlane)?;
    if plane.width != expected.width
        || plane.height != expected.height
        || (!high_precision && plane.bits_per_pixel != 8)
        || (high_precision && !(9..=16).contains(&plane.bits_per_pixel))
    {
        return Err(CodecError::InvalidPixelLayout {
            width: expected.width,
            height: expected.height,
            stride: plane.stride,
        });
    }
    let profile = if let Some(raw) = handle.color_profile_raw() {
        check(
            CodecLimit::ColourProfileBytes,
            raw.data.len() as u64,
            MAX_PROFILE.into(),
        )?;
        let profile = ColorProfile::new_from_slice_with_options(
            &raw.data,
            ParsingOptions {
                max_profile_size: MAX_PROFILE as usize + 1,
                max_allowed_clut_size: 1_000_000,
                max_allowed_trc_size: 40_000,
            },
        )
        .map_err(decode_error)?;
        Some(profile)
    } else if let Some(nclx) = image
        .color_profile_nclx()
        .or_else(|| handle.color_profile_nclx())
    {
        let cicp = CicpProfile {
            color_primaries: (nclx.color_primaries() as u8)
                .try_into()
                .map_err(decode_error)?,
            transfer_characteristics: (nclx.transfer_characteristics() as u8)
                .try_into()
                .map_err(decode_error)?,
            matrix_coefficients: MatrixCoefficients::Identity,
            full_range: true,
        };
        let profile = ColorProfile::new_from_cicp(cicp);
        if profile.cicp.is_none() || profile.red_trc.is_none() {
            return Err(decode_error("unsupported NCLX colour profile"));
        }
        Some(profile)
    } else {
        None
    };
    let mut pixels = if high_precision {
        copy_high_precision_rows(
            plane.data,
            expected.width,
            expected.height,
            plane.stride,
            plane.bits_per_pixel,
            profile.as_ref(),
        )?
    } else {
        copy_rgb_rows(plane.data, expected.width, expected.height, plane.stride)?
    };
    // The high-precision HDR path already performed transfer/gamut/tone mapping.
    if let Some(profile) = profile.as_ref()
        && (!high_precision || crate::heif_hdr::HdrColour::from_profile(profile)?.is_none())
    {
        convert_to_srgb(&mut pixels, expected.width, profile)?;
    }
    let rgb = RgbImage::from_raw(expected.width, expected.height, pixels).ok_or(
        CodecError::InvalidPixelLayout {
            width: expected.width,
            height: expected.height,
            stride: plane.stride,
        },
    )?;
    Ok(DynamicImage::ImageRgb8(rgb))
}

fn copy_rgb_rows(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
) -> Result<Vec<u8>, CodecError> {
    let invalid = || CodecError::InvalidPixelLayout {
        width,
        height,
        stride,
    };
    let row = (width as usize).checked_mul(3).ok_or_else(invalid)?;
    let length = row.checked_mul(height as usize).ok_or_else(invalid)?;
    if width == 0 || height == 0 || stride < row {
        return Err(invalid());
    }
    let end = (height as usize - 1)
        .checked_mul(stride)
        .and_then(|n| n.checked_add(row))
        .ok_or_else(invalid)?;
    if end > data.len() {
        return Err(invalid());
    }
    check(CodecLimit::AllocationBytes, length as u64, MAX_ALLOCATION)?;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(length).map_err(decode_error)?;
    for y in 0..height as usize {
        pixels.extend_from_slice(&data[y * stride..y * stride + row]);
    }
    Ok(pixels)
}

fn convert_to_srgb(
    pixels: &mut [u8],
    width: u32,
    profile: &ColorProfile,
) -> Result<(), CodecError> {
    if let Some(hdr) = crate::heif_hdr::HdrColour::from_profile(profile)? {
        for pixel in pixels.chunks_exact_mut(3) {
            let encoded = [pixel[0], pixel[1], pixel[2]].map(|v| f64::from(v) / 255.0);
            pixel.copy_from_slice(&hdr.pixel(encoded));
        }
        return Ok(());
    }
    let transform = profile
        .create_transform_8bit(
            Layout::Rgb,
            &ColorProfile::new_srgb(),
            Layout::Rgb,
            Default::default(),
        )
        .map_err(decode_error)?;
    let mut output = Vec::new();
    output
        .try_reserve_exact(width as usize * 3)
        .map_err(decode_error)?;
    output.resize(width as usize * 3, 0);
    for row in pixels.chunks_exact_mut(output.len()) {
        transform
            .transform(row, &mut output)
            .map_err(decode_error)?;
        row.copy_from_slice(&output);
    }
    Ok(())
}

fn copy_high_precision_rows(
    data: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    bits: u8,
    profile: Option<&ColorProfile>,
) -> Result<Vec<u8>, CodecError> {
    let invalid = || CodecError::InvalidPixelLayout {
        width,
        height,
        stride,
    };
    let row = (width as usize).checked_mul(6).ok_or_else(invalid)?;
    let length = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))
        .ok_or_else(invalid)?;
    if width == 0 || height == 0 || stride < row || !(9..=16).contains(&bits) {
        return Err(invalid());
    }
    let end = (height as usize - 1)
        .checked_mul(stride)
        .and_then(|n| n.checked_add(row))
        .ok_or_else(invalid)?;
    if end > data.len() {
        return Err(invalid());
    }
    check(CodecLimit::AllocationBytes, length as u64, MAX_ALLOCATION)?;
    let hdr = profile
        .map(crate::heif_hdr::HdrColour::from_profile)
        .transpose()?
        .flatten();
    let maximum = (1u32 << bits) - 1;
    let mut pixels = Vec::new();
    pixels.try_reserve_exact(length).map_err(decode_error)?;
    for y in 0..height as usize {
        for pixel in data[y * stride..y * stride + row].chunks_exact(6) {
            let values = [
                u16::from_le_bytes([pixel[0], pixel[1]]),
                u16::from_le_bytes([pixel[2], pixel[3]]),
                u16::from_le_bytes([pixel[4], pixel[5]]),
            ];
            if values.iter().any(|v| u32::from(*v) > maximum) {
                return Err(invalid());
            }
            let rgb = if let Some(hdr) = hdr.as_ref() {
                hdr.pixel(values.map(|v| f64::from(v) / f64::from(maximum)))
            } else {
                // SDR uses the native bit-shift reduction. Performing YCbCr
                // conversion before quantization can improve rounding by one
                // code value; HDR remains precise through luminance mapping.
                values.map(|v| (v >> (bits - 8)) as u8)
            };
            pixels.extend_from_slice(&rgb);
        }
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pq_ten_bit_plane_preserves_distinct_shadow_codes_and_stride() {
        // ST 2084: codes 100 and 103 (10-bit) independently map to sRGB
        // 10 and 11. Reducing both to code 25 (8-bit) would erase this detail.
        let bytes = [100, 0, 100, 0, 100, 0, 99, 99, 103, 0, 103, 0, 103, 0];
        let pixels =
            copy_high_precision_rows(&bytes, 1, 2, 8, 10, Some(&ColorProfile::new_bt2020_pq()))
                .unwrap();
        assert_eq!(pixels, [10, 10, 10, 11, 11, 11]);
    }

    #[test]
    fn pq_reference_white_and_highlights_use_absolute_luminance() {
        // ST 2084 independently gives 101.75 / 1010.3 / 10000 nits.
        let mut pixels = [130, 130, 130, 192, 192, 192, 255, 255, 255];
        convert_to_srgb(&mut pixels, 3, &ColorProfile::new_bt2020_pq()).unwrap();
        for (actual, expected) in pixels
            .into_iter()
            .zip([225u8, 225, 225, 252, 252, 252, 255, 255, 255])
        {
            assert!(
                actual.abs_diff(expected) <= 1,
                "actual {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn hlg_reference_display_and_highlights_use_system_gamma() {
        let mut pixels = [64, 64, 64, 160, 160, 160, 255, 255, 255];
        convert_to_srgb(&mut pixels, 3, &ColorProfile::new_bt2020_hlg()).unwrap();
        for (actual, expected) in pixels
            .into_iter()
            .zip([88u8, 88, 88, 224, 224, 224, 252, 252, 252])
        {
            assert!(
                actual.abs_diff(expected) <= 1,
                "actual {actual}, expected {expected}"
            );
        }
    }

    #[test]
    fn stride_padding_is_discarded_without_requiring_last_row_padding() {
        assert_eq!(
            copy_rgb_rows(&[1, 2, 3, 99, 99, 4, 5, 6], 1, 2, 5).unwrap(),
            [1, 2, 3, 4, 5, 6]
        );
        for (bytes, width, height, stride) in [
            (&[0u8; 5][..], 1, 2, 3),
            (&[0u8; 6][..], 1, 2, 2),
            (&[][..], 0, 1, 0),
        ] {
            assert!(matches!(
                copy_rgb_rows(bytes, width, height, stride),
                Err(CodecError::InvalidPixelLayout { .. })
            ));
        }
    }
}
