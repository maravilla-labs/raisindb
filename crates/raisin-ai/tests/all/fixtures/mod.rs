//! Test fixtures for raisin-ai tests.
//!
//! Provides utilities to generate test images and PDFs programmatically.

use image::{DynamicImage, Rgb, RgbImage};

/// Create a test image with a simple pattern.
///
/// Returns raw JPEG bytes suitable for CLIP/BLIP processing.
pub fn create_test_jpeg(width: u32, height: u32) -> Vec<u8> {
    let mut img = RgbImage::new(width, height);

    // Create a gradient pattern
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 255) / width) as u8;
            let g = ((y * 255) / height) as u8;
            let b = 128u8;
            img.put_pixel(x, y, Rgb([r, g, b]));
        }
    }

    // Encode to JPEG
    let mut buffer = std::io::Cursor::new(Vec::new());
    let dynamic = DynamicImage::ImageRgb8(img);
    dynamic
        .write_to(&mut buffer, image::ImageFormat::Jpeg)
        .expect("Failed to encode JPEG");

    buffer.into_inner()
}

/// Create a test image with a solid color.
pub fn create_solid_color_jpeg(width: u32, height: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
    let mut img = RgbImage::new(width, height);

    for y in 0..height {
        for x in 0..width {
            img.put_pixel(x, y, Rgb([r, g, b]));
        }
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    let dynamic = DynamicImage::ImageRgb8(img);
    dynamic
        .write_to(&mut buffer, image::ImageFormat::Jpeg)
        .expect("Failed to encode JPEG");

    buffer.into_inner()
}

/// Create a test PNG image with transparency.
pub fn create_test_png(width: u32, height: u32) -> Vec<u8> {
    use image::{Rgba, RgbaImage};

    let mut img = RgbaImage::new(width, height);

    // Create a pattern with varying alpha
    for y in 0..height {
        for x in 0..width {
            let r = ((x * 255) / width) as u8;
            let g = ((y * 255) / height) as u8;
            let b = 128u8;
            let a = (((x + y) * 255) / (width + height)) as u8;
            img.put_pixel(x, y, Rgba([r, g, b, a]));
        }
    }

    let mut buffer = std::io::Cursor::new(Vec::new());
    let dynamic = DynamicImage::ImageRgba8(img);
    dynamic
        .write_to(&mut buffer, image::ImageFormat::Png)
        .expect("Failed to encode PNG");

    buffer.into_inner()
}

/// Minimal valid PDF with "Hello World" text.
///
/// This is a hand-crafted minimal PDF that pdf-extract can parse.
/// Contains a single page with the text "Hello World from PDF test".
pub fn minimal_pdf_with_text() -> Vec<u8> {
    // A minimal valid PDF with actual text content
    // This PDF has been carefully crafted to work with pdf-extract
    let pdf = r#"%PDF-1.4
1 0 obj
<</Type /Catalog /Pages 2 0 R>>
endobj
2 0 obj
<</Type /Pages /Kids [3 0 R] /Count 1>>
endobj
3 0 obj
<</Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources <</Font <</F1 5 0 R>>>>>>
endobj
4 0 obj
<</Length 89>>
stream
BT
/F1 24 Tf
100 700 Td
(Hello World from PDF test document) Tj
ET
endstream
endobj
5 0 obj
<</Type /Font /Subtype /Type1 /BaseFont /Helvetica>>
endobj
xref
0 6
0000000000 65535 f
0000000009 00000 n
0000000058 00000 n
0000000115 00000 n
0000000266 00000 n
0000000406 00000 n
trailer
<</Size 6 /Root 1 0 R>>
startxref
478
%%EOF"#;

    pdf.as_bytes().to_vec()
}

/// Create a minimal PDF with multiple pages.
pub fn minimal_multi_page_pdf() -> Vec<u8> {
    let pdf = r#"%PDF-1.4
1 0 obj
<</Type /Catalog /Pages 2 0 R>>
endobj
2 0 obj
<</Type /Pages /Kids [3 0 R 6 0 R] /Count 2>>
endobj
3 0 obj
<</Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources <</Font <</F1 5 0 R>>>>>>
endobj
4 0 obj
<</Length 55>>
stream
BT
/F1 24 Tf
100 700 Td
(Page One Content) Tj
ET
endstream
endobj
5 0 obj
<</Type /Font /Subtype /Type1 /BaseFont /Helvetica>>
endobj
6 0 obj
<</Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 7 0 R /Resources <</Font <</F1 5 0 R>>>>>>
endobj
7 0 obj
<</Length 55>>
stream
BT
/F1 24 Tf
100 700 Td
(Page Two Content) Tj
ET
endstream
endobj
xref
0 8
0000000000 65535 f
0000000009 00000 n
0000000058 00000 n
0000000122 00000 n
0000000273 00000 n
0000000379 00000 n
0000000451 00000 n
0000000602 00000 n
trailer
<</Size 8 /Root 1 0 R>>
startxref
708
%%EOF"#;

    pdf.as_bytes().to_vec()
}

/// Create a PDF that appears to be scanned (minimal text per page).
///
/// This simulates a PDF with only a few characters per page,
/// triggering the "likely scanned" heuristic.
pub fn minimal_scanned_pdf() -> Vec<u8> {
    let pdf = r#"%PDF-1.4
1 0 obj
<</Type /Catalog /Pages 2 0 R>>
endobj
2 0 obj
<</Type /Pages /Kids [3 0 R] /Count 1>>
endobj
3 0 obj
<</Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources <</Font <</F1 5 0 R>>>>>>
endobj
4 0 obj
<</Length 30>>
stream
BT
/F1 12 Tf
100 700 Td
(.) Tj
ET
endstream
endobj
5 0 obj
<</Type /Font /Subtype /Type1 /BaseFont /Helvetica>>
endobj
xref
0 6
0000000000 65535 f
0000000009 00000 n
0000000058 00000 n
0000000115 00000 n
0000000266 00000 n
0000000347 00000 n
trailer
<</Size 6 /Root 1 0 R>>
startxref
419
%%EOF"#;

    pdf.as_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_jpeg() {
        let jpeg = create_test_jpeg(100, 100);
        assert!(!jpeg.is_empty());
        // Check JPEG magic bytes
        assert_eq!(&jpeg[0..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn test_create_png() {
        let png = create_test_png(100, 100);
        assert!(!png.is_empty());
        // Check PNG magic bytes
        assert_eq!(&png[0..8], b"\x89PNG\r\n\x1a\n");
    }

    #[test]
    fn test_minimal_pdf_format() {
        let pdf = minimal_pdf_with_text();
        assert!(!pdf.is_empty());
        // Check PDF magic bytes
        assert!(pdf.starts_with(b"%PDF-"));
    }
}
