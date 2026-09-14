use std::{io, path::Path};

use image::{DynamicImage, ImageError, ImageReader};
use photo_domain::MediaKind;

use crate::{CodecError, DisplayShape, IMAGE_DECODER_FINGERPRINT};

pub fn display_shape(path: &Path, kind: MediaKind) -> Result<DisplayShape, CodecError> {
    ensure_image_backend(kind)?;
    let size = imagesize::size(path).map_err(|error| size_error(path, error))?;
    let width = u32::try_from(size.width).map_err(|_| CodecError::Decode {
        message: "image width exceeds u32".into(),
    })?;
    let height = u32::try_from(size.height).map_err(|_| CodecError::Decode {
        message: "image height exceeds u32".into(),
    })?;
    Ok(DisplayShape { width, height })
}

pub fn decode_display_image(
    path: &Path,
    kind: MediaKind,
    orientation: u16,
) -> Result<DynamicImage, CodecError> {
    ensure_image_backend(kind)?;
    let image = ImageReader::open(path)
        .map_err(|error| io_error(path, error))?
        .with_guessed_format()
        .map_err(|error| io_error(path, error))?
        .decode()
        .map_err(|error| image_error(path, error))?;
    Ok(apply_orientation(image, orientation))
}

pub fn decoder_fingerprint(kind: MediaKind) -> Result<&'static str, CodecError> {
    ensure_image_backend(kind)?;
    Ok(IMAGE_DECODER_FINGERPRINT)
}

fn ensure_image_backend(kind: MediaKind) -> Result<(), CodecError> {
    match kind {
        MediaKind::Jpeg | MediaKind::Png | MediaKind::Tiff | MediaKind::Webp => Ok(()),
        MediaKind::Heif => {
            #[cfg(feature = "heic")]
            return Err(CodecError::Unsupported { kind });
            #[cfg(not(feature = "heic"))]
            Err(CodecError::Unsupported { kind })
        }
        MediaKind::Raw | MediaKind::Avif | MediaKind::Video | MediaKind::Unknown => {
            Err(CodecError::Unsupported { kind })
        }
    }
}

fn image_error(path: &Path, error: ImageError) -> CodecError {
    match error {
        ImageError::IoError(error) => CodecError::Io {
            path: path.to_owned(),
            kind: error.kind(),
            message: error.to_string(),
        },
        error => CodecError::Decode {
            message: error.to_string(),
        },
    }
}

fn io_error(path: &Path, error: io::Error) -> CodecError {
    CodecError::Io {
        path: path.to_owned(),
        kind: error.kind(),
        message: error.to_string(),
    }
}

fn size_error(path: &Path, error: imagesize::ImageError) -> CodecError {
    match error {
        imagesize::ImageError::IoError(error) => io_error(path, error),
        error => CodecError::Decode {
            message: error.to_string(),
        },
    }
}

fn apply_orientation(image: DynamicImage, orientation: u16) -> DynamicImage {
    match orientation {
        2 => image.fliph(),
        3 => image.rotate180(),
        4 => image.flipv(),
        5 => image.fliph().rotate270(),
        6 => image.rotate90(),
        7 => image.fliph().rotate90(),
        8 => image.rotate270(),
        _ => image,
    }
}
