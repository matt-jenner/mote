use std::io::BufReader;
use std::path::Path;

use chrono::{FixedOffset, NaiveDateTime, TimeZone};
use exif::{In, Tag, Value};

use crate::{MetadataBundle, MetadataCandidate, MetadataReadWarning, MetadataSource};

pub struct EmbeddedExifReader;

impl EmbeddedExifReader {
    pub fn read(path: &Path) -> Result<MetadataBundle, MetadataReadWarning> {
        if photo_domain::MediaKind::from_path(path) == Some(photo_domain::MediaKind::Heif) {
            return match photo_codec::embedded_exif_tiff(path, photo_domain::MediaKind::Heif)
                .map_err(|error| MetadataReadWarning::new("exif_read_failed", error.to_string()))?
            {
                Some(tiff) => Self::read_tiff(&tiff),
                None => Ok(MetadataBundle::default()),
            };
        }
        let file = std::fs::File::open(path)
            .map_err(|error| MetadataReadWarning::new("exif_open_failed", error.to_string()))?;
        let mut reader = BufReader::new(file);
        let exif = exif::Reader::new()
            .read_from_container(&mut reader)
            .map_err(|error| MetadataReadWarning::new("exif_read_failed", error.to_string()))?;
        Ok(bundle_from_exif(&exif))
    }

    pub fn read_tiff(data: &[u8]) -> Result<MetadataBundle, MetadataReadWarning> {
        let exif = exif::Reader::new()
            .read_raw(data.to_vec())
            .map_err(|error| MetadataReadWarning::new("exif_read_failed", error.to_string()))?;
        Ok(bundle_from_exif(&exif))
    }
}

fn bundle_from_exif(exif: &exif::Exif) -> MetadataBundle {
    let orientation = exif
        .get_field(Tag::Orientation, In::PRIMARY)
        .and_then(|field| field.value.get_uint(0))
        .and_then(|value| u16::try_from(value).ok());
    let mut bundle = MetadataBundle {
        orientation,
        ..MetadataBundle::default()
    };
    add_date(
        &mut bundle,
        exif,
        Tag::DateTimeOriginal,
        MetadataSource::EmbeddedExifOriginal,
    );
    add_date(
        &mut bundle,
        exif,
        Tag::DateTimeDigitized,
        MetadataSource::EmbeddedExifDigitized,
    );
    add_date(
        &mut bundle,
        exif,
        Tag::DateTime,
        MetadataSource::EmbeddedExif,
    );
    bundle
}

fn add_date(bundle: &mut MetadataBundle, exif: &exif::Exif, tag: Tag, source: MetadataSource) {
    let Some(field) = exif.get_field(tag, In::PRIMARY) else {
        return;
    };
    let Some(raw) = ascii_value(&field.value) else {
        return;
    };
    let parsed = NaiveDateTime::parse_from_str(raw.trim(), "%Y:%m:%d %H:%M:%S")
        .ok()
        .and_then(|value| {
            FixedOffset::east_opt(0)?
                .from_local_datetime(&value)
                .single()
        });
    if let Some(value) = parsed {
        bundle.capture_dates.push(MetadataCandidate {
            value,
            source,
            raw_value: raw,
        });
    } else {
        bundle.warnings.push(crate::MetadataWarning {
            code: "invalid_exif_date",
            message: format!("invalid {tag:?} value {raw:?}"),
        });
    }
}

fn ascii_value(value: &Value) -> Option<String> {
    let Value::Ascii(values) = value else {
        return None;
    };
    let bytes = values.first()?;
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8(bytes[..end].to_vec()).ok()
}
