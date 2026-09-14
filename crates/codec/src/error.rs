use std::{io, path::PathBuf};

use photo_domain::MediaKind;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecLimit {
    EncodedBytes,
    DecodedPixels,
    ItemCount,
    TileCount,
    ColourProfileBytes,
    AllocationBytes,
    TotalMemoryBytes,
}

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("I/O error reading {path}: {message}")]
    Io {
        path: PathBuf,
        kind: io::ErrorKind,
        message: String,
    },
    #[error("unsupported media kind: {kind:?}")]
    Unsupported { kind: MediaKind },
    #[error("{limit:?} limit exceeded: {actual} exceeds {maximum}")]
    LimitExceeded {
        limit: CodecLimit,
        actual: u64,
        maximum: u64,
    },
    #[error("container error: {message}")]
    Container { message: String },
    #[error("decode error: {message}")]
    Decode { message: String },
    #[error("missing primary image")]
    MissingPrimaryImage,
    #[error("missing interleaved plane")]
    MissingInterleavedPlane,
    #[error("invalid pixel layout for {width}x{height} image with stride {stride}")]
    InvalidPixelLayout {
        width: u32,
        height: u32,
        stride: usize,
    },
    #[error("invalid EXIF: {message}")]
    InvalidExif { message: String },
}
