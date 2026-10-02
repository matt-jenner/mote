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
        let media_kind = photo_domain::MediaKind::from_path(media_path);
        let mut bundle = match EmbeddedExifReader::read(media_path) {
            Ok(bundle) => bundle,
            Err(error)
                if matches!(error.code, "exif_open_failed" | "exif_io_failed")
                    || media_kind == Some(photo_domain::MediaKind::Heif)
                        && matches!(
                            error.code,
                            "source_missing" | "source_unreadable" | "source_check_failed"
                        ) =>
            {
                return Err(error);
            }
            Err(error) => MetadataBundle {
                warnings: vec![photo_metadata::MetadataWarning {
                    code: error.code,
                    message: error.message,
                }],
                ..MetadataBundle::default()
            },
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
    fn read_time_io_is_retryable_for_jpeg_and_heif() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["photo.heic", "photo.jpg"] {
            let path = temp.path().join(name);
            std::fs::create_dir(&path).unwrap();
            std::fs::File::open(&path).unwrap().metadata().unwrap();
            assert!(
                matches!(
                    DefaultMetadataReader.read(&path, None).unwrap_err().code,
                    "source_check_failed" | "exif_open_failed" | "exif_io_failed"
                ),
                "{name} read-time I/O must remain retryable"
            );
        }
    }

    #[test]
    fn absent_and_malformed_jpeg_exif_remain_nonfatal() {
        let temp = tempfile::tempdir().unwrap();
        let plain = temp.path().join("plain.jpg");
        image::RgbImage::from_pixel(2, 2, image::Rgb([1, 2, 3]))
            .save(&plain)
            .unwrap();
        assert!(DefaultMetadataReader.read(&plain, None).is_ok());

        let malformed = temp.path().join("malformed.jpg");
        std::fs::write(&malformed, b"not a jpeg").unwrap();
        let bundle = DefaultMetadataReader.read(&malformed, None).unwrap();
        assert!(
            bundle
                .warnings
                .iter()
                .any(|warning| warning.code == "exif_read_failed")
        );
    }
}
