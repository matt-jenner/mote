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

pub(crate) fn heif_codec_warning(
    error: photo_codec::CodecError,
    terminal_code: &'static str,
) -> MetadataReadWarning {
    use std::io::ErrorKind;
    let code = match &error {
        photo_codec::CodecError::Io {
            kind: ErrorKind::NotFound | ErrorKind::NotADirectory,
            ..
        } => "source_missing",
        photo_codec::CodecError::Io {
            kind: ErrorKind::PermissionDenied,
            ..
        } => "source_unreadable",
        photo_codec::CodecError::Io { .. } => "source_check_failed",
        _ => terminal_code,
    };
    MetadataReadWarning::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn typed_heif_io_keeps_retry_semantics_without_reopening_the_path() {
        use photo_codec::CodecError;
        use std::io::ErrorKind;
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("readable.heic");
        std::fs::write(&path, b"readable").unwrap();
        for (kind, expected) in [
            (ErrorKind::NotFound, "source_missing"),
            (ErrorKind::PermissionDenied, "source_unreadable"),
            (ErrorKind::Other, "source_check_failed"),
            (ErrorKind::Interrupted, "source_check_failed"),
        ] {
            std::fs::File::open(&path).unwrap().metadata().unwrap();
            for terminal in ["shape_read_failed", "exif_read_failed"] {
                let warning = super::heif_codec_warning(
                    CodecError::Io {
                        path: path.clone(),
                        kind,
                        message: "read-time I/O failure".into(),
                    },
                    terminal,
                );
                assert_eq!(warning.code, expected);
            }
        }
        assert_eq!(
            super::heif_codec_warning(
                CodecError::InvalidExif {
                    message: "bad TIFF".into()
                },
                "exif_read_failed"
            )
            .code,
            "exif_read_failed"
        );
    }
}
