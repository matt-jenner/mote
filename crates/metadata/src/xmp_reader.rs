use std::io::Read;
use std::path::Path;

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

        loop {
            match reader.read_event_into(&mut buffer) {
                Ok(Event::Start(start)) => {
                    depth += 1;
                    update_context_on_start(start.name().as_ref(), &mut keyword_context);
                    if is_name(start.name().as_ref(), b"rdf:li", b"li") {
                        reading_keyword = keyword_context.is_some();
                    }
                    read_rating(&start, &mut bundle)?;
                }
                Ok(Event::Empty(start)) => read_rating(&start, &mut bundle)?,
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
                Ok(Event::End(end)) => {
                    if is_name(end.name().as_ref(), b"rdf:li", b"li") {
                        reading_keyword = false;
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
