#![forbid(unsafe_code)]

mod error;
#[cfg(feature = "heic")]
mod heif_backend;
mod image_backend;

pub const IMAGE_DECODER_FINGERPRINT: &str = "image-0.25-v1";
#[cfg(feature = "heic")]
pub const HEIC_DECODER_FINGERPRINT: &str = "libheif-1.23.4-libde265-1.1.1-sdr-v1";

pub use error::{CodecError, CodecLimit};
pub use photo_domain::MediaKind;

pub fn display_shape(path: &std::path::Path, kind: MediaKind) -> Result<DisplayShape, CodecError> {
    #[cfg(feature = "heic")]
    if kind == MediaKind::Heif {
        return heif_backend::display_shape(path);
    }
    image_backend::display_shape(path, kind)
}

pub fn decode_display_image(
    path: &std::path::Path,
    kind: MediaKind,
    orientation: u16,
) -> Result<image::DynamicImage, CodecError> {
    #[cfg(feature = "heic")]
    if kind == MediaKind::Heif {
        return heif_backend::decode(path);
    }
    image_backend::decode_display_image(path, kind, orientation)
}

pub fn decoder_fingerprint(kind: MediaKind) -> Result<&'static str, CodecError> {
    #[cfg(feature = "heic")]
    if kind == MediaKind::Heif {
        return Ok(HEIC_DECODER_FINGERPRINT);
    }
    image_backend::decoder_fingerprint(kind)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayShape {
    pub width: u32,
    pub height: u32,
}
