use std::io::Read;
use std::path::Path;

use chrono::{DateTime, FixedOffset, NaiveDateTime, TimeZone};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

use crate::{
    KeywordCandidate, MetadataBundle, MetadataCandidate, MetadataReadWarning, MetadataSource,
    MetadataWarning,
};

const MAX_SIDECAR_BYTES: usize = 16 * 1024 * 1024;
const MAX_TEXT_BYTES: usize = 16 * 1024;

pub struct XmpSidecarReader;

#[derive(Clone, Copy)]
enum KeywordContext {
    Flat,
    Hierarchical,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum CaptureDateKind {
    Original,
    DateCreated,
    CreateDate,
}

#[derive(Default)]
struct CaptureDateValues {
    original: Option<String>,
    date_created: Option<String>,
    create_date: Option<String>,
}

impl CaptureDateValues {
    fn set(&mut self, kind: CaptureDateKind, value: &str) {
        let slot = match kind {
            CaptureDateKind::Original => &mut self.original,
            CaptureDateKind::DateCreated => &mut self.date_created,
            CaptureDateKind::CreateDate => &mut self.create_date,
        };
        if slot.is_none() {
            *slot = Some(value.trim().to_owned());
        }
    }
}

impl XmpSidecarReader {
    pub fn read_path(path: &Path) -> Result<MetadataBundle, MetadataReadWarning> {
        let file = std::fs::File::open(path)
            .map_err(|error| MetadataReadWarning::new("xmp_open_failed", error.to_string()))?;
        let mut bytes = Vec::new();
        file.take((MAX_SIDECAR_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| MetadataReadWarning::new("xmp_read_failed", error.to_string()))?;
        Self::read(&bytes)
    }

    pub fn read(bytes: &[u8]) -> Result<MetadataBundle, MetadataReadWarning> {
        if bytes.len() > MAX_SIDECAR_BYTES {
            return Err(MetadataReadWarning::new(
                "oversized_xmp",
                format!("sidecar exceeds {MAX_SIDECAR_BYTES} bytes"),
            ));
        }

        let mut reader = Reader::from_reader(bytes);
        reader.config_mut().trim_text(true);
        let mut buffer = Vec::new();
        let mut bundle = MetadataBundle::default();
        let mut depth = 0_usize;
        let mut keyword_context = None;
        let mut reading_keyword = false;
        let mut reading_capture_date = None;
        let mut capture_dates = CaptureDateValues::default();

        loop {
            match reader.read_event_into(&mut buffer) {
                Ok(Event::Start(start)) => {
                    depth += 1;
                    update_context_on_start(start.name().as_ref(), &mut keyword_context);
                    if is_name(start.name().as_ref(), b"rdf:li", b"li") {
                        reading_keyword = keyword_context.is_some();
                    }
                    reading_capture_date = capture_date_kind(start.name().as_ref());
                    read_rating(&start, &mut bundle)?;
                    read_capture_date_attributes(&start, &mut capture_dates)?;
                }
                Ok(Event::Empty(start)) => {
                    read_rating(&start, &mut bundle)?;
                    read_capture_date_attributes(&start, &mut capture_dates)?;
                }
                Ok(Event::Text(text)) if reading_keyword => {
                    if text.as_ref().len() > MAX_TEXT_BYTES {
                        bundle.warnings.push(MetadataWarning {
                            code: "oversized_xmp_value",
                            message: format!("XMP text exceeds {MAX_TEXT_BYTES} bytes"),
                        });
                    } else {
                        let value = text.decode().map_err(|error| {
                            MetadataReadWarning::new("malformed_xmp", error.to_string())
                        })?;
                        add_keyword(&mut bundle, value.trim(), keyword_context);
                    }
                }
                Ok(Event::Text(text)) if reading_capture_date.is_some() => {
                    if text.as_ref().len() > MAX_TEXT_BYTES {
                        bundle.warnings.push(MetadataWarning {
                            code: "oversized_xmp_value",
                            message: format!("XMP text exceeds {MAX_TEXT_BYTES} bytes"),
                        });
                    } else {
                        let value = text.decode().map_err(|error| {
                            MetadataReadWarning::new("malformed_xmp", error.to_string())
                        })?;
                        capture_dates.set(
                            reading_capture_date.expect("capture date context checked"),
                            value.trim(),
                        );
                    }
                }
                Ok(Event::End(end)) => {
                    if is_name(end.name().as_ref(), b"rdf:li", b"li") {
                        reading_keyword = false;
                    }
                    if capture_date_kind(end.name().as_ref()).is_some() {
                        reading_capture_date = None;
                    }
                    update_context_on_end(end.name().as_ref(), &mut keyword_context);
                    depth = depth.saturating_sub(1);
                }
                Ok(Event::Eof) if depth == 0 => break,
                Ok(Event::Eof) => {
                    return Err(MetadataReadWarning::new(
                        "malformed_xmp",
                        "sidecar ended before all elements were closed",
                    ));
                }
                Ok(_) => {}
                Err(error) => {
                    return Err(MetadataReadWarning::new("malformed_xmp", error.to_string()));
                }
            }
            buffer.clear();
        }
        append_capture_dates(&mut bundle, capture_dates);
        Ok(bundle)
    }
}

fn read_rating(
    start: &BytesStart<'_>,
    bundle: &mut MetadataBundle,
) -> Result<(), MetadataReadWarning> {
    for attribute in start.attributes() {
        let attribute = attribute
            .map_err(|error| MetadataReadWarning::new("malformed_xmp", error.to_string()))?;
        if !is_name(attribute.key.as_ref(), b"xmp:Rating", b"Rating") {
            continue;
        }
        let raw = std::str::from_utf8(attribute.value.as_ref())
            .map_err(|error| MetadataReadWarning::new("malformed_xmp", error.to_string()))?;
        match raw.parse::<i16>() {
            Ok(value @ 0..=5) => bundle.ratings.push(MetadataCandidate {
                value: value as u8,
                source: MetadataSource::SidecarXmp,
                raw_value: raw.to_owned(),
            }),
            _ => bundle.warnings.push(MetadataWarning {
                code: "invalid_rating",
                message: format!("invalid XMP rating {raw:?}"),
            }),
        }
    }
    Ok(())
}

fn read_capture_date_attributes(
    start: &BytesStart<'_>,
    values: &mut CaptureDateValues,
) -> Result<(), MetadataReadWarning> {
    for attribute in start.attributes() {
        let attribute = attribute
            .map_err(|error| MetadataReadWarning::new("malformed_xmp", error.to_string()))?;
        let Some(kind) = capture_date_kind(attribute.key.as_ref()) else {
            continue;
        };
        let raw = std::str::from_utf8(attribute.value.as_ref())
            .map_err(|error| MetadataReadWarning::new("malformed_xmp", error.to_string()))?;
        values.set(kind, raw);
    }
    Ok(())
}

fn append_capture_dates(bundle: &mut MetadataBundle, values: CaptureDateValues) {
    for raw in [values.original, values.date_created, values.create_date]
        .into_iter()
        .flatten()
    {
        match parse_capture_date(&raw) {
            Some(value) => bundle.capture_dates.push(MetadataCandidate {
                value,
                source: MetadataSource::SidecarXmp,
                raw_value: raw,
            }),
            None => bundle.warnings.push(MetadataWarning {
                code: "invalid_xmp_date",
                message: format!("invalid XMP capture date {raw:?}"),
            }),
        }
    }
}

fn parse_capture_date(raw: &str) -> Option<DateTime<FixedOffset>> {
    if let Ok(value) = DateTime::parse_from_rfc3339(raw) {
        return Some(value);
    }
    [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y:%m:%d %H:%M:%S%.f",
    ]
    .into_iter()
    .find_map(|format| NaiveDateTime::parse_from_str(raw, format).ok())
    .and_then(|value| {
        FixedOffset::east_opt(0)?
            .from_local_datetime(&value)
            .single()
    })
}

fn capture_date_kind(name: &[u8]) -> Option<CaptureDateKind> {
    if is_name(name, b"exif:DateTimeOriginal", b"DateTimeOriginal") {
        Some(CaptureDateKind::Original)
    } else if is_name(name, b"photoshop:DateCreated", b"DateCreated") {
        Some(CaptureDateKind::DateCreated)
    } else if is_name(name, b"xmp:CreateDate", b"CreateDate") {
        Some(CaptureDateKind::CreateDate)
    } else {
        None
    }
}

fn update_context_on_start(name: &[u8], context: &mut Option<KeywordContext>) {
    if is_name(name, b"dc:subject", b"subject") {
        *context = Some(KeywordContext::Flat);
    } else if is_name(name, b"lr:hierarchicalSubject", b"hierarchicalSubject") {
        *context = Some(KeywordContext::Hierarchical);
    }
}

fn update_context_on_end(name: &[u8], context: &mut Option<KeywordContext>) {
    if is_name(name, b"dc:subject", b"subject")
        || is_name(name, b"lr:hierarchicalSubject", b"hierarchicalSubject")
    {
        *context = None;
    }
}

fn add_keyword(bundle: &mut MetadataBundle, value: &str, context: Option<KeywordContext>) {
    if value.is_empty() {
        return;
    }
    match context {
        Some(KeywordContext::Flat) => bundle.keywords.push(KeywordCandidate::new(
            value,
            None,
            MetadataSource::SidecarXmp,
        )),
        Some(KeywordContext::Hierarchical) => {
            let display = value.rsplit('|').next().unwrap_or(value);
            bundle.keywords.push(KeywordCandidate::new(
                display,
                Some(value),
                MetadataSource::SidecarXmp,
            ));
        }
        None => {}
    }
}

fn is_name(actual: &[u8], qualified: &[u8], local: &[u8]) -> bool {
    actual == qualified || actual == local
}
