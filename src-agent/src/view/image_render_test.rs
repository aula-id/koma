use super::*;

#[test]
fn render_to_lines_basic() {
    // Create a tiny 2x2 red image.
    let img = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
    let mut buf = std::io::Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    let bytes = buf.into_inner();

    let tmp = std::env::temp_dir().join("koma_test_render.png");
    std::fs::write(&tmp, &bytes).unwrap();

    let result = ImageRenderer::render_to_lines(&tmp, 80);
    assert!(result.is_ok());
    let lines = result.unwrap();
    assert!(!lines.is_empty());

    let _ = std::fs::remove_file(tmp);
}
