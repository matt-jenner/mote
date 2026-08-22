use chrono::{DateTime, FixedOffset};
use photo_metadata::{
    KeywordCandidate, MetadataBundle, MetadataCandidate, MetadataResolver, MetadataSource,
};

fn rating(value: u8, source: MetadataSource) -> MetadataCandidate<u8> {
    MetadataCandidate {
        value,
        source,
        raw_value: value.to_string(),
    }
}

fn date(value: &str, source: MetadataSource) -> MetadataCandidate<DateTime<FixedOffset>> {
    MetadataCandidate {
        value: DateTime::parse_from_rfc3339(value).unwrap(),
        source,
        raw_value: value.to_owned(),
    }
}

#[test]
fn sidecar_rating_beats_embedded_and_path_rating() {
    let bundle = MetadataBundle {
        ratings: vec![
            rating(3, MetadataSource::PathPolicy),
            rating(4, MetadataSource::EmbeddedXmp),
            rating(2, MetadataSource::SidecarXmp),
        ],
        ..MetadataBundle::default()
    };

    let resolved = MetadataResolver::resolve(bundle);

    assert_eq!(resolved.rating, Some(2));
    assert_eq!(
        resolved
            .provenance
            .iter()
            .filter(|record| record.field == "rating" && record.chosen)
            .count(),
        1
    );
}

#[test]
fn keywords_union_case_insensitively_and_preserve_hierarchy() {
    let bundle = MetadataBundle {
        keywords: vec![
            KeywordCandidate::new("Family", None, MetadataSource::EmbeddedIptc),
            KeywordCandidate::new(" family ", None, MetadataSource::SidecarXmp),
            KeywordCandidate::new(
                "London",
                Some("Places|UK|London"),
                MetadataSource::SidecarXmp,
            ),
        ],
        ..MetadataBundle::default()
    };

    let resolved = MetadataResolver::resolve(bundle);

    assert_eq!(resolved.keywords.len(), 2);
    assert_eq!(resolved.keywords[0].normalized, "family");
    assert_eq!(resolved.keywords[0].display_value, "Family");
    assert_eq!(
        resolved.keywords[1].hierarchy.as_deref(),
        Some("Places|UK|London")
    );
}

#[test]
fn distinct_hierarchies_for_the_same_keyword_are_retained() {
    let bundle = MetadataBundle {
        keywords: vec![
            KeywordCandidate::new(
                "London",
                Some("Places|UK|London"),
                MetadataSource::EmbeddedXmp,
            ),
            KeywordCandidate::new(" london ", Some("Trips|London"), MetadataSource::SidecarXmp),
        ],
        ..MetadataBundle::default()
    };

    let resolved = MetadataResolver::resolve(bundle);

    assert_eq!(resolved.keywords.len(), 2);
    assert_eq!(resolved.keywords[0].display_value, "London");
    assert_eq!(resolved.keywords[1].display_value, "London");
}

#[test]
fn invalid_ratings_become_warnings_and_are_never_selected() {
    let resolved = MetadataResolver::resolve(MetadataBundle {
        ratings: vec![
            rating(9, MetadataSource::SidecarXmp),
            rating(4, MetadataSource::PathPolicy),
        ],
        ..MetadataBundle::default()
    });

    assert_eq!(resolved.rating, Some(4));
    assert_eq!(resolved.warnings.len(), 1);
    assert_eq!(resolved.warnings[0].code, "invalid_rating");
}

#[test]
fn capture_date_uses_field_specific_source_order() {
    let original = "2020-01-02T03:04:05+00:00";
    let digitized = "2021-01-02T03:04:05+00:00";
    let container = "2022-01-02T03:04:05+00:00";
    let birth = "2023-01-02T03:04:05+00:00";
    let modified = "2024-01-02T03:04:05+00:00";
    let bundle = MetadataBundle {
        capture_dates: vec![
            date(modified, MetadataSource::FilesystemModified),
            date(container, MetadataSource::Container),
            date(digitized, MetadataSource::EmbeddedExifDigitized),
            date(birth, MetadataSource::FilesystemBirth),
            date(original, MetadataSource::EmbeddedExifOriginal),
        ],
        ..MetadataBundle::default()
    };

    let resolved = MetadataResolver::resolve(bundle);

    assert_eq!(resolved.captured_at.unwrap().to_rfc3339(), original);
}

#[test]
fn filesystem_modified_is_the_final_capture_date_fallback() {
    let modified = "2024-01-02T03:04:05+00:00";
    let resolved = MetadataResolver::resolve(MetadataBundle {
        capture_dates: vec![date(modified, MetadataSource::FilesystemModified)],
        ..MetadataBundle::default()
    });

    assert_eq!(resolved.captured_at.unwrap().to_rfc3339(), modified);
}
