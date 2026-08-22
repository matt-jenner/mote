use std::path::Path;

use crate::MetadataReadWarning;

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
        let size = imagesize::size(path)
            .map_err(|error| MetadataReadWarning::new("shape_read_failed", format!("{error}")))?;
        let width = u32::try_from(size.width)
            .map_err(|_| MetadataReadWarning::new("shape_too_large", "width exceeds u32"))?;
        let height = u32::try_from(size.height)
            .map_err(|_| MetadataReadWarning::new("shape_too_large", "height exceeds u32"))?;
        Ok(ImageShape { width, height })
    }

    pub fn representative_rgb(path: &Path) -> Result<RepresentativeRgb, MetadataReadWarning> {
        let image = image::ImageReader::open(path)
            .map_err(|error| MetadataReadWarning::new("image_open_failed", error.to_string()))?
            .with_guessed_format()
            .map_err(|error| MetadataReadWarning::new("image_format_failed", error.to_string()))?
            .decode()
            .map_err(|error| MetadataReadWarning::new("image_decode_failed", error.to_string()))?;
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
