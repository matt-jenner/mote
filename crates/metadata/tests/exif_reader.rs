#![cfg(feature = "heic")]

use std::path::{Path, PathBuf};

use photo_metadata::{EmbeddedExifReader, MetadataResolver, MetadataSource, XmpSidecarReader};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../codec/tests/fixtures/heif")
        .join(name)
}

#[test]
fn heif_exif_original_keeps_existing_capture_date_precedence() {
    let mut bundle = EmbeddedExifReader::read(&fixture("portrait-rotated.heic")).unwrap();
    bundle.extend(
        XmpSidecarReader::read(
            br#"<rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/"
                xmp:CreateDate="2030-01-02T03:04:05Z"/>"#,
        )
        .unwrap(),
    );

    assert!(bundle.capture_dates.iter().any(|candidate| {
        candidate.source == MetadataSource::EmbeddedExifOriginal
            && candidate.raw_value == "2024:03:04 05:06:07"
    }));
    let resolved = MetadataResolver::resolve(bundle);
    assert_eq!(
        resolved.captured_at.unwrap().to_rfc3339(),
        "2024-03-04T05:06:07+00:00"
    );
}

#[test]
fn heif_without_exif_is_an_empty_embedded_bundle() {
    let bundle = EmbeddedExifReader::read(&fixture("iphone-8bit.heic")).unwrap();
    assert!(bundle.capture_dates.is_empty());
    assert!(bundle.warnings.is_empty());
    assert_eq!(bundle.orientation, None);
}
