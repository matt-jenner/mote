mod exif_reader;
mod model;
mod probe;
mod resolve;
mod xmp_reader;

pub use exif_reader::EmbeddedExifReader;
pub use model::{
    Keyword, KeywordCandidate, MetadataBundle, MetadataCandidate, MetadataSource, MetadataWarning,
    ProvenanceRecord, ResolvedMetadata,
};
pub use probe::{ImageShape, MediaProbe, RepresentativeRgb};
pub use resolve::MetadataResolver;
pub use xmp_reader::XmpSidecarReader;

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
#[error("{code}: {message}")]
pub struct MetadataReadWarning {
    pub code: &'static str,
    pub message: String,
}

impl MetadataReadWarning {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
