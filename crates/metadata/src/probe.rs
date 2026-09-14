use std::path::Path;

use crate::MetadataReadWarning;
use photo_codec::{CodecError, MediaKind, decode_display_image, display_shape};

pub struct MediaProbe;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ImageShape {
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RepresentativeRgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

impl MediaProbe {
    pub fn shape(path: &Path) -> Result<ImageShape, MetadataReadWarning> {
        let shape = display_shape(path, media_kind(path)).map_err(shape_warning)?;
        Ok(ImageShape {
            width: shape.width,
            height: shape.height,
        })
    }

    pub fn representative_rgb(path: &Path) -> Result<RepresentativeRgb, MetadataReadWarning> {
        let image = decode_display_image(path, media_kind(path), 1).map_err(decode_warning)?;
        let image = if image.width() > 32 || image.height() > 32 {
            image.thumbnail(32, 32)
        } else {
            image
        }
        .to_rgb8();
        let count = u64::from(image.width()) * u64::from(image.height());
        if count == 0 {
            return Err(MetadataReadWarning::new(
                "empty_image",
                "decoded image contains no pixels",
            ));
        }
        let sums = image.pixels().fold([0_u64; 3], |mut sums, pixel| {
            sums[0] += u64::from(pixel[0]);
            sums[1] += u64::from(pixel[1]);
            sums[2] += u64::from(pixel[2]);
            sums
        });
        Ok(RepresentativeRgb {
            red: ((sums[0] + count / 2) / count) as u8,
            green: ((sums[1] + count / 2) / count) as u8,
            blue: ((sums[2] + count / 2) / count) as u8,
        })
    }
}

fn media_kind(path: &Path) -> MediaKind {
    MediaKind::from_path(path).unwrap_or(MediaKind::Unknown)
}

fn shape_warning(error: CodecError) -> MetadataReadWarning {
    MetadataReadWarning::new("shape_read_failed", error.to_string())
}

fn decode_warning(error: CodecError) -> MetadataReadWarning {
    let code = if matches!(error, CodecError::Io { .. }) {
        "image_open_failed"
    } else {
        "image_decode_failed"
    };
    MetadataReadWarning::new(code, error.to_string())
}
