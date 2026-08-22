use std::collections::{HashMap, HashSet};

use crate::{
    Keyword, MetadataBundle, MetadataSource, MetadataWarning, ProvenanceRecord, ResolvedMetadata,
};

const RATING_PRECEDENCE: &[MetadataSource] = &[
    MetadataSource::SidecarXmp,
    MetadataSource::EmbeddedXmp,
    MetadataSource::EmbeddedExif,
    MetadataSource::EmbeddedIptc,
    MetadataSource::PathPolicy,
];

const CAPTURE_DATE_PRECEDENCE: &[MetadataSource] = &[
    MetadataSource::EmbeddedExifOriginal,
    MetadataSource::SidecarXmp,
    MetadataSource::EmbeddedXmp,
    MetadataSource::EmbeddedExifDigitized,
    MetadataSource::EmbeddedExif,
    MetadataSource::EmbeddedIptc,
    MetadataSource::Container,
    MetadataSource::FilesystemBirth,
    MetadataSource::FilesystemModified,
];

pub struct MetadataResolver;

impl MetadataResolver {
    pub fn resolve(bundle: MetadataBundle) -> ResolvedMetadata {
        let MetadataBundle {
            capture_dates,
            ratings,
            keywords,
            orientation: _,
            mut warnings,
        } = bundle;
        let mut provenance = Vec::new();

        let chosen_rating = choose_index(
            &ratings
                .iter()
                .enumerate()
                .filter(|(_, candidate)| candidate.value <= 5)
                .map(|(index, candidate)| (index, candidate.source))
                .collect::<Vec<_>>(),
            RATING_PRECEDENCE,
        );
        for (index, candidate) in ratings.iter().enumerate() {
            if candidate.value > 5 {
                warnings.push(MetadataWarning {
                    code: "invalid_rating",
                    message: format!(
                        "rating {} from {:?} is outside 0..=5",
                        candidate.value, candidate.source
                    ),
                });
            }
            provenance.push(ProvenanceRecord {
                field: "rating".to_owned(),
                source: candidate.source,
                raw_value: candidate.raw_value.clone(),
                chosen: chosen_rating == Some(index),
            });
        }

        let chosen_date = choose_index(
            &capture_dates
                .iter()
                .enumerate()
                .map(|(index, candidate)| (index, candidate.source))
                .collect::<Vec<_>>(),
            CAPTURE_DATE_PRECEDENCE,
        );
        for (index, candidate) in capture_dates.iter().enumerate() {
            provenance.push(ProvenanceRecord {
                field: "captured_at".to_owned(),
                source: candidate.source,
                raw_value: candidate.raw_value.clone(),
                chosen: chosen_date == Some(index),
            });
        }

        let mut resolved_keywords = Vec::new();
        let mut seen = HashSet::new();
        let mut display_values = HashMap::new();
        for candidate in keywords {
            let normalized = normalize(&candidate.value);
            if normalized.is_empty() {
                warnings.push(MetadataWarning {
                    code: "empty_keyword",
                    message: format!("empty keyword from {:?}", candidate.source),
                });
                provenance.push(ProvenanceRecord {
                    field: "keyword".to_owned(),
                    source: candidate.source,
                    raw_value: candidate.value,
                    chosen: false,
                });
                continue;
            }

            let normalized_hierarchy = candidate.hierarchy.as_deref().map(normalize);
            let identity = (normalized.clone(), normalized_hierarchy);
            let chosen = seen.insert(identity);
            let display = display_values
                .entry(normalized.clone())
                .or_insert_with(|| collapse_whitespace(&candidate.value))
                .clone();
            if chosen {
                resolved_keywords.push(Keyword {
                    normalized,
                    display_value: display,
                    hierarchy: candidate
                        .hierarchy
                        .as_deref()
                        .map(collapse_whitespace)
                        .filter(|value| !value.is_empty()),
                });
            }
            provenance.push(ProvenanceRecord {
                field: "keyword".to_owned(),
                source: candidate.source,
                raw_value: candidate.value,
                chosen,
            });
        }

        ResolvedMetadata {
            captured_at: chosen_date.map(|index| capture_dates[index].value),
            rating: chosen_rating.map(|index| ratings[index].value),
            keywords: resolved_keywords,
            provenance,
            warnings,
        }
    }
}

fn choose_index(
    candidates: &[(usize, MetadataSource)],
    precedence: &[MetadataSource],
) -> Option<usize> {
    precedence.iter().find_map(|source| {
        candidates
            .iter()
            .find_map(|(index, candidate_source)| (candidate_source == source).then_some(*index))
    })
}

fn normalize(value: &str) -> String {
    collapse_whitespace(value).to_lowercase()
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}
