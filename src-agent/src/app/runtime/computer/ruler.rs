//! Edge ruler burned into the model observation only. Tick numbers are screenshot
//! pixels, which is the coordinate space `computer_act` uses. `x` and `y` are
//! desktop pixels per screenshot pixel on each axis. The live preview is untouched.
use super::contract::Transform;
use image::{Rgba, RgbaImage};

const SCALE: i32 = 2;
const GLYPH_W: i32 = 5;
const GLYPH_H: i32 = 7;
const ADVANCE: i32 = GLYPH_W * SCALE + SCALE;

/// Skip frames too small to spare an edge. A dock strip would be covered by the type.
const MIN_WIDTH: u32 = 240;
const MIN_HEIGHT: u32 = 160;

pub(crate) fn stamp(png: &[u8], transform: &Transform) -> Option<Vec<u8>> {
    if transform.width < MIN_WIDTH
        || transform.height < MIN_HEIGHT
        || transform.desktop.width <= 0.0
        || transform.desktop.height <= 0.0
    {
        return None;
    }
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    if image.width() != transform.width || image.height() != transform.height {
        return None;
    }
    let mut img = image.to_rgba8();
    let sx = transform.desktop.width / f64::from(transform.width);
    let sy = transform.desktop.height / f64::from(transform.height);
    let caption = format!("x{sx:.3} y{sy:.3}");
    let caption_w = text_width(&caption);
    let caption_x = img.width() as i32 - caption_w - 4;
    if caption_x < ADVANCE * 4 {
        return None;
    }
    let mut white = Vec::new();
    let mut black = Vec::new();
    horizontal(&img, caption_x, &mut white);
    vertical(&img, &mut white);
    queue_text(&caption, caption_x, 1, &mut white);
    for (x, y) in white {
        halo(&img, x, y, &mut black);
    }
    for (x, y) in &black {
        plot(&mut img, *x, *y, Rgba([0, 0, 0, 255]));
    }
    // Recompute the white pixels; halo must not erase the glyph.
    let mut white = Vec::new();
    horizontal(&img, caption_x, &mut white);
    vertical(&img, &mut white);
    queue_text(&caption, caption_x, 1, &mut white);
    for (x, y) in white {
        plot(&mut img, x, y, Rgba([255, 255, 255, 255]));
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    img.write_to(&mut encoded, image::ImageFormat::Png).ok()?;
    Some(encoded.into_inner())
}

fn horizontal(img: &RgbaImage, stop: i32, out: &mut Vec<(i32, i32)>) {
    let width = img.width() as i32;
    let mut x = 0;
    while x < stop && x < width {
        let major = x % 100 == 0;
        let reach = if major { 8 } else { 4 };
        if x % 50 == 0 {
            for y in 0..reach {
                out.push((x, y));
            }
        }
        if major {
            queue_text(&x.to_string(), x + 2, 1, out);
        }
        x += 50;
    }
}

fn vertical(img: &RgbaImage, out: &mut Vec<(i32, i32)>) {
    let height = img.height() as i32;
    let mut y = 100;
    while y < height {
        for x in 0..8 {
            out.push((x, y));
        }
        let label = y.to_string();
        let top = y - GLYPH_H * SCALE / 2;
        if top > GLYPH_H * SCALE + 2 {
            queue_text(&label, 2, top, out);
        }
        y += 100;
    }
}

fn queue_text(text: &str, x: i32, y: i32, out: &mut Vec<(i32, i32)>) {
    let mut cursor = x;
    for ch in text.chars() {
        let rows = glyph(ch);
        for (row, bits) in rows.iter().enumerate() {
            for col in 0..GLYPH_W {
                if bits & (1 << (GLYPH_W - 1 - col)) == 0 {
                    continue;
                }
                for dy in 0..SCALE {
                    for dx in 0..SCALE {
                        out.push((cursor + col * SCALE + dx, y + row as i32 * SCALE + dy));
                    }
                }
            }
        }
        cursor += ADVANCE;
    }
}

fn text_width(text: &str) -> i32 {
    let n = text.chars().count() as i32;
    if n == 0 {
        0
    } else {
        n * ADVANCE - SCALE
    }
}

fn halo(img: &RgbaImage, x: i32, y: i32, out: &mut Vec<(i32, i32)>) {
    for dy in -1..=1 {
        for dx in -1..=1 {
            let px = x + dx;
            let py = y + dy;
            if px >= 0 && py >= 0 && (px as u32) < img.width() && (py as u32) < img.height() {
                out.push((px, py));
            }
        }
    }
}

fn plot(img: &mut RgbaImage, x: i32, y: i32, color: Rgba<u8>) {
    if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
        img.put_pixel(x as u32, y as u32, color);
    }
}

/// 5×7 glyphs, top row first, high bit on the left. Only the ruler alphabet.
fn glyph(ch: char) -> [u8; 7] {
    match ch {
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x02, 0x04, 0x08, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        'x' => [0x00, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x00],
        'y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        _ => [0; 7],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::runtime::computer::contract::{Rect, Transform};

    fn frame(width: u32, height: u32, desktop_w: f64, desktop_h: f64) -> (Vec<u8>, Transform) {
        let image = RgbaImage::from_pixel(width, height, Rgba([20, 40, 60, 255]));
        let mut png = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        (
            png,
            Transform {
                desktop: Rect {
                    x: 0.0,
                    y: 0.0,
                    width: desktop_w,
                    height: desktop_h,
                },
                width,
                height,
            },
        )
    }

    #[test]
    fn ruler_keeps_the_click_grid_and_paints_both_scales() {
        let (png, transform) = frame(320, 200, 640.0, 500.0);
        let stamped = stamp(&png, &transform).unwrap();
        let image = image::load_from_memory(&stamped).unwrap().to_rgba8();
        assert_eq!((image.width(), image.height()), (320, 200));
        assert_eq!(image.get_pixel(160, 100).0, [20, 40, 60, 255]);
        assert_ne!(image.get_pixel(100, 0).0, [20, 40, 60, 255]);
        assert_ne!(image.get_pixel(0, 100).0, [20, 40, 60, 255]);
        let caption = "x2.000 y2.500";
        let origin = 320 - text_width(caption) - 4;
        let mut marked = false;
        for y in 1..16 {
            for x in origin..origin + 10 {
                if image.get_pixel(x as u32, y as u32).0[0] == 255 {
                    marked = true;
                }
            }
        }
        assert!(
            marked,
            "horizontal and vertical scale caption is on the frame"
        );
    }

    #[test]
    fn small_frames_are_left_alone() {
        let (png, transform) = frame(80, 40, 80.0, 40.0);
        assert!(stamp(&png, &transform).is_none());
    }
}
