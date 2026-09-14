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
    let mut options =
        DecodingOptions::new().ok_or_else(|| decode_error("cannot allocate decoding options"))?;
    options.set_ignore_transformations(false);
    options.set_strict_decoding(true);
    options.set_convert_hdr_to_8bit(true);
    // libheif converts YCbCr layout/range and bit depth, but is not an ICC
    // colour-management engine. Preserve source colourimetry for moxcms.
    options.set_output_image_nclx_profile_passthrough(true);
    options
        .set_decoder_id(Some("libde265"))
        .map_err(decode_error)?;
    let image = LIBRARY
        .get_or_init(LibHeif::new)
        .decode(&handle, ColorSpace::Rgb(RgbChroma::Rgb), Some(options))
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
    if plane.width != expected.width || plane.height != expected.height || plane.bits_per_pixel != 8
    {
        return Err(CodecError::InvalidPixelLayout {
            width: expected.width,
            height: expected.height,
            stride: plane.stride,
        });
    }
    let mut pixels = copy_rgb_rows(plane.data, expected.width, expected.height, plane.stride)?;
    if let Some(raw) = handle.color_profile_raw() {
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
        convert_to_srgb(&mut pixels, expected.width, &profile)?;
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
        convert_to_srgb(&mut pixels, expected.width, &profile)?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
