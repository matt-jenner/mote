#![forbid(unsafe_code)]

mod error;
mod image_backend;

pub const IMAGE_DECODER_FINGERPRINT: &str = "image-0.25-v1";
#[cfg(feature = "heic")]
pub const HEIC_DECODER_FINGERPRINT: &str = "libheif-1.23.4-libde265-1.1.1-sdr-v1";

pub use error::{CodecError, CodecLimit};
pub use image_backend::{decode_display_image, decoder_fingerprint, display_shape};
pub use photo_domain::MediaKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DisplayShape {
    pub width: u32,
    pub height: u32,
}
