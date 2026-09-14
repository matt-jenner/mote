#![forbid(unsafe_code)]

pub const IMAGE_DECODER_FINGERPRINT: &str = "image-0.25-v1";
#[cfg(feature = "heic")]
pub const HEIC_DECODER_FINGERPRINT: &str = "libheif-1.23.4-libde265-1.1.1-sdr-v1";
