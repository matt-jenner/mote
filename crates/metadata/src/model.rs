use chrono::{DateTime, FixedOffset};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum MetadataSource {
    SidecarXmp,
    EmbeddedXmp,
    EmbeddedExifOriginal,
    EmbeddedExifDigitized,
    EmbeddedExif,
    EmbeddedIptc,
    Container,
    FilesystemBirth,
    FilesystemModified,
    PathPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataCandidate<T> {
    pub value: T,
    pub source: MetadataSource,
    pub raw_value: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MetadataBundle {
    pub capture_dates: Vec<MetadataCandidate<DateTime<FixedOffset>>>,
    pub ratings: Vec<MetadataCandidate<u8>>,
    pub keywords: Vec<KeywordCandidate>,
    pub orientation: Option<u16>,
    pub warnings: Vec<MetadataWarning>,
}

impl MetadataBundle {
    pub fn extend(&mut self, other: MetadataBundle) {
        self.capture_dates.extend(other.capture_dates);
        self.ratings.extend(other.ratings);
        self.keywords.extend(other.keywords);
        if self.orientation.is_none() {
            self.orientation = other.orientation;
        }
        self.warnings.extend(other.warnings);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeywordCandidate {
    pub value: String,
    pub hierarchy: Option<String>,
    pub source: MetadataSource,
}

impl KeywordCandidate {
    pub fn new(value: impl Into<String>, hierarchy: Option<&str>, source: MetadataSource) -> Self {
        Self {
            value: value.into(),
            hierarchy: hierarchy.map(str::to_owned),
            source,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataWarning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Keyword {
    pub normalized: String,
    pub display_value: String,
    pub hierarchy: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProvenanceRecord {
    pub field: String,
    pub source: MetadataSource,
    pub raw_value: String,
    pub chosen: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResolvedMetadata {
    pub captured_at: Option<DateTime<FixedOffset>>,
    pub rating: Option<u8>,
    pub keywords: Vec<Keyword>,
    pub provenance: Vec<ProvenanceRecord>,
    pub warnings: Vec<MetadataWarning>,
}
