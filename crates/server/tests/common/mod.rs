use std::io::Cursor;

pub fn jpeg_bytes(rgb: [u8; 3]) -> Vec<u8> {
    let image = image::RgbImage::from_fn(32, 24, |x, y| {
        image::Rgb([
            rgb[0].saturating_add((x % 24) as u8),
            rgb[1].saturating_add((y % 24) as u8),
            rgb[2],
        ])
    });
    let mut output = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut output, image::ImageFormat::Jpeg)
        .unwrap();
    output.into_inner()
}
