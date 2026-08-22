use photo_metadata::{EmbeddedExifReader, MediaProbe, RepresentativeRgb, XmpSidecarReader};

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
