use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use photo_metadata::{
    EmbeddedExifReader, MetadataBundle, MetadataCandidate, MetadataReadWarning, MetadataSource,
    XmpSidecarReader,
};

use crate::MetadataReader;

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultMetadataReader;

impl MetadataReader for DefaultMetadataReader {
    fn read(
        &self,
        media_path: &Path,
        sidecar_path: Option<&Path>,
    ) -> Result<MetadataBundle, MetadataReadWarning> {
        let mut bundle = match EmbeddedExifReader::read(media_path) {
            Err(error)
                if photo_domain::MediaKind::from_path(media_path)
                    == Some(photo_domain::MediaKind::Heif)
                    && matches!(
                        error.code,
                        "source_missing" | "source_unreadable" | "source_check_failed"
                    ) =>
            {
                return Err(error);
            }
            result => result.unwrap_or_default(),
        };
        if let Some(sidecar) = sidecar_path {
            match XmpSidecarReader::read_path(sidecar) {
                Ok(value) => bundle.extend(value),
                Err(error) => bundle.warnings.push(photo_metadata::MetadataWarning {
                    code: error.code,
                    message: error.message,
                }),
            }
        }
        if let Ok(metadata) = media_path.metadata() {
            if let Ok(created) = metadata.created()
                && let Some(value) = to_datetime(created)
            {
                let raw_value = value.to_rfc3339();
                bundle.capture_dates.push(MetadataCandidate {
                    value,
                    source: MetadataSource::FilesystemBirth,
                    raw_value,
                });
            }
            if let Ok(modified) = metadata.modified()
                && let Some(value) = to_datetime(modified)
            {
                let raw_value = value.to_rfc3339();
                bundle.capture_dates.push(MetadataCandidate {
                    value,
                    source: MetadataSource::FilesystemModified,
                    raw_value,
                });
            }
        }
        Ok(bundle)
    }
}

fn to_datetime(value: SystemTime) -> Option<chrono::DateTime<chrono::FixedOffset>> {
    let seconds = value.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    chrono::DateTime::from_timestamp(seconds, 0).map(|value| value.fixed_offset())
}

#[cfg(all(test, feature = "heic", unix))]
mod tests {
    use super::*;

    #[test]
    fn read_time_io_is_retryable_for_heif_without_changing_jpeg_tolerance() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["photo.heic", "photo.jpg"] {
            let path = temp.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::File::open(&path).unwrap().metadata().unwrap();
            let result = DefaultMetadataReader.read(&path, None);
            if name.ends_with("heic") {
                assert_eq!(result.unwrap_err().code, "source_check_failed");
            } else {
                assert!(!result.unwrap().capture_dates.is_empty());
            }
        }
    }
}
