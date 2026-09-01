use photo_metadata::{
    EmbeddedExifReader, MediaProbe, MetadataSource, RepresentativeRgb, XmpSidecarReader,
};

#[test]
fn reads_rating_and_hierarchical_keywords_from_xmp() {
    let xml = br#"<x:xmpmeta xmlns:x="adobe:ns:meta/">
      <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
        <rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/"
          xmlns:dc="http://purl.org/dc/elements/1.1/"
          xmlns:lr="http://ns.adobe.com/lightroom/1.0/" xmp:Rating="4">
          <dc:subject><rdf:Bag><rdf:li>Family</rdf:li></rdf:Bag></dc:subject>
          <lr:hierarchicalSubject><rdf:Bag><rdf:li>Places|UK|London</rdf:li></rdf:Bag></lr:hierarchicalSubject>
        </rdf:Description>
      </rdf:RDF>
    </x:xmpmeta>"#;

    let bundle = XmpSidecarReader::read(xml).unwrap();

    assert_eq!(bundle.ratings[0].value, 4);
    assert!(
        bundle
            .keywords
            .iter()
            .any(|keyword| keyword.value == "Family")
    );
    assert!(
        bundle
            .keywords
            .iter()
            .any(|keyword| { keyword.hierarchy.as_deref() == Some("Places|UK|London") })
    );
}

#[test]
fn reads_sidecar_capture_dates_in_capture_priority_order() {
    let xml = br#"<rdf:Description
      xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
      xmlns:xmp="http://ns.adobe.com/xap/1.0/"
      xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
      xmlns:exif="http://ns.adobe.com/exif/1.0/"
      xmp:CreateDate="2024-01-02T03:04:05Z"
      xmp:ModifyDate="2025-02-03T04:05:06Z"
      xmp:MetadataDate="2026-03-04T05:06:07Z"
      photoshop:DateCreated="2023-01-02T03:04:05+01:00"
      exif:DateTimeOriginal="2022-01-02T03:04:05"/>"#;

    let bundle = XmpSidecarReader::read(xml).unwrap();

    assert_eq!(bundle.capture_dates.len(), 3);
    assert_eq!(
        bundle
            .capture_dates
            .iter()
            .map(|candidate| candidate.raw_value.as_str())
            .collect::<Vec<_>>(),
        vec![
            "2022-01-02T03:04:05",
            "2023-01-02T03:04:05+01:00",
            "2024-01-02T03:04:05Z",
        ]
    );
    assert!(
        bundle
            .capture_dates
            .iter()
            .all(|candidate| candidate.source == MetadataSource::SidecarXmp)
    );
    assert_eq!(
        bundle.capture_dates[0].value.to_rfc3339(),
        "2022-01-02T03:04:05+00:00"
    );
    assert_eq!(
        bundle.capture_dates[1].value.to_rfc3339(),
        "2023-01-02T03:04:05+01:00"
    );
}

#[test]
fn reads_sidecar_capture_dates_from_element_text_and_skips_invalid_values() {
    let xml = br#"<rdf:Description
      xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
      xmlns:xmp="http://ns.adobe.com/xap/1.0/"
      xmlns:photoshop="http://ns.adobe.com/photoshop/1.0/"
      xmlns:exif="http://ns.adobe.com/exif/1.0/">
      <xmp:CreateDate>2024-01-02T03:04:05.123Z</xmp:CreateDate>
      <photoshop:DateCreated>not-a-date</photoshop:DateCreated>
      <exif:DateTimeOriginal>2022:01:02 03:04:05</exif:DateTimeOriginal>
    </rdf:Description>"#;

    let bundle = XmpSidecarReader::read(xml).unwrap();

    assert_eq!(bundle.capture_dates.len(), 2);
    assert_eq!(
        bundle
            .capture_dates
            .iter()
            .map(|candidate| candidate.raw_value.as_str())
            .collect::<Vec<_>>(),
        vec!["2022:01:02 03:04:05", "2024-01-02T03:04:05.123Z"]
    );
    assert_eq!(bundle.warnings.len(), 1);
    assert_eq!(bundle.warnings[0].code, "invalid_xmp_date");
}

#[test]
fn capture_date_fields_require_their_namespace_uri() {
    let xml = br#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <rdf:Description
        xmlns:camera="http://ns.adobe.com/exif/1.0/"
        camera:DateTimeOriginal="2022-01-02T03:04:05Z"/>
      <rdf:Description
        xmlns:xmp="urn:not-adobe-xmp"
        xmp:CreateDate="2025-01-02T03:04:05Z"
        CreateDate="2026-01-02T03:04:05Z"/>
    </rdf:RDF>"#;

    let bundle = XmpSidecarReader::read(xml).unwrap();

    assert_eq!(bundle.capture_dates.len(), 1);
    assert_eq!(bundle.capture_dates[0].raw_value, "2022-01-02T03:04:05Z");
}

#[test]
fn a_valid_duplicate_capture_date_follows_an_invalid_value() {
    let xml = br#"<rdf:Description
      xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
      xmlns:xmp="http://ns.adobe.com/xap/1.0/">
      <xmp:CreateDate>not-a-date</xmp:CreateDate>
      <xmp:CreateDate>2024-01-02T03:04:05Z</xmp:CreateDate>
    </rdf:Description>"#;

    let bundle = XmpSidecarReader::read(xml).unwrap();

    assert_eq!(bundle.capture_dates.len(), 1);
    assert_eq!(bundle.capture_dates[0].raw_value, "2024-01-02T03:04:05Z");
    assert_eq!(bundle.warnings.len(), 1);
    assert_eq!(bundle.warnings[0].code, "invalid_xmp_date");
}

#[test]
fn reads_orientation_from_minimal_little_endian_tiff() {
    let tiff = [
        0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00, 0x01,
        0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    let bundle = EmbeddedExifReader::read_tiff(&tiff).unwrap();

    assert_eq!(bundle.orientation, Some(6));
}

#[test]
fn representative_colour_averages_a_small_rgb_image() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("two-pixels.png");
    let image = image::RgbImage::from_raw(2, 1, vec![255, 0, 0, 0, 0, 255]).unwrap();
    image.save(&path).unwrap();

    let shape = MediaProbe::shape(&path).unwrap();
    let colour = MediaProbe::representative_rgb(&path).unwrap();

    assert_eq!((shape.width, shape.height), (2, 1));
    assert_eq!(
        colour,
        RepresentativeRgb {
            red: 128,
            green: 0,
            blue: 128,
        }
    );
}

#[test]
fn malformed_and_oversized_sidecars_return_typed_warnings() {
    let malformed = XmpSidecarReader::read(b"<rdf:RDF><rdf:Description>").unwrap_err();
    let oversized = XmpSidecarReader::read(&vec![b' '; 16 * 1024 * 1024 + 1]).unwrap_err();

    assert_eq!(malformed.code, "malformed_xmp");
    assert_eq!(oversized.code, "oversized_xmp");
}

#[test]
fn invalid_xmp_rating_is_a_warning_not_a_candidate() {
    let xml = br#"<rdf:Description xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
        xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="9"/>"#;

    let bundle = XmpSidecarReader::read(xml).unwrap();

    assert!(bundle.ratings.is_empty());
    assert_eq!(bundle.warnings[0].code, "invalid_rating");
}

#[test]
fn oversized_xmp_text_is_skipped_without_aborting_the_sidecar() {
    let long_keyword = "x".repeat(16 * 1024 + 1);
    let xml = format!(
        "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:subject><rdf:Bag><rdf:li>{long_keyword}</rdf:li></rdf:Bag></dc:subject></rdf:RDF>"
    );

    let bundle = XmpSidecarReader::read(xml.as_bytes()).unwrap();

    assert!(bundle.keywords.is_empty());
    assert_eq!(bundle.warnings[0].code, "oversized_xmp_value");
}
