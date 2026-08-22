mod model;
mod resolve;

pub use model::{
    Keyword, KeywordCandidate, MetadataBundle, MetadataCandidate, MetadataSource, MetadataWarning,
    ProvenanceRecord, ResolvedMetadata,
};
pub use resolve::MetadataResolver;
