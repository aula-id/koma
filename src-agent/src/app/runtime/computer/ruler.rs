//! Margin ruler around the model observation. Tick numbers sit outside the
//! page and are screen pixels: the window or display origin plus the screenshot
//! pixel times the scale. That printed number is what `computer_act` sends.
//! A light grid on the page marks the same values. The margin is not a click
//! surface. The live preview is untouched.
use super::contract::Transform;
use image::{Rgba, RgbaImage};

const SCALE: i32 = 2;
const GLYPH_W: i32 = 5;
const ADVANCE: i32 = GLYPH_W * SCALE + SCALE;

/// Skip frames too small to spare an edge. A dock strip would be covered by the type.
const MIN_WIDTH: u32 = 240;
const MIN_HEIGHT: u32 = 160;
/// Room for a screen coordinate such as `-7680` beside the page.
pub(crate) const MARGIN_LEFT: u32 = 92;
pub(crate) const MARGIN_TOP: u32 = 22;
const MARGIN_RIGHT: u32 = 8;
const MARGIN_BOTTOM: u32 = 8;

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
    let page = image.to_rgba8();
    let width = page.width();
    let height = page.height();
    let canvas_w = width + MARGIN_LEFT + MARGIN_RIGHT;
    let canvas_h = height + MARGIN_TOP + MARGIN_BOTTOM;
    let mut img = RgbaImage::from_pixel(canvas_w, canvas_h, Rgba([24, 24, 24, 255]));
    for y in 0..height {
        for x in 0..width {
            img.put_pixel(MARGIN_LEFT + x, MARGIN_TOP + y, *page.get_pixel(x, y));
        }
    }
    draw_grid(&mut img, width, height);
    let mut white = Vec::new();
    let mut black = Vec::new();
    labels(&img, transform, &mut white);
    let caption = format!(
        "x{:.3} y{:.3}",
        transform.desktop.width / f64::from(width),
        transform.desktop.height / f64::from(height)
    );
    let caption_w = text_width(&caption);
    let caption_x = canvas_w as i32 - caption_w - 4;
    if caption_x > MARGIN_LEFT as i32 {
        queue_text(&caption, caption_x, 4, &mut white);
    }
    for (x, y) in &white {
        halo(&img, *x, *y, &mut black);
    }
    for (x, y) in &black {
        if in_page(&img, *x, *y, width, height) {
            continue;
        }
        plot(&mut img, *x, *y, Rgba([0, 0, 0, 255]));
    }
    for (x, y) in white {
        if in_page(&img, x, y, width, height) {
            continue;
        }
        plot(&mut img, x, y, Rgba([255, 255, 255, 255]));
    }
    let mut encoded = std::io::Cursor::new(Vec::new());
    img.write_to(&mut encoded, image::ImageFormat::Png).ok()?;
    Some(encoded.into_inner())
}

fn in_page(img: &RgbaImage, x: i32, y: i32, width: u32, height: u32) -> bool {
    x >= MARGIN_LEFT as i32
        && y >= MARGIN_TOP as i32
        && x < MARGIN_LEFT as i32 + width as i32
        && y < MARGIN_TOP as i32 + height as i32
        && x >= 0
        && y >= 0
        && (x as u32) < img.width()
        && (y as u32) < img.height()
}

fn draw_grid(img: &mut RgbaImage, width: u32, height: u32) {
    let mut x = 0;
    while x < width {
        let strong = x % 100 == 0;
        if x % 50 == 0 {
            for y in 0..height {
                mark(img, MARGIN_LEFT + x, MARGIN_TOP + y, strong);
            }
        }
        x += 50;
    }
    let mut y = 0;
    while y < height {
        let strong = y % 100 == 0;
        if y % 50 == 0 {
            for x in 0..width {
                mark(img, MARGIN_LEFT + x, MARGIN_TOP + y, strong);
            }
        }
        y += 50;
    }
}

fn mark(img: &mut RgbaImage, x: u32, y: u32, strong: bool) {
    if x >= img.width() || y >= img.height() {
        return;
    }
    let pixel = *img.get_pixel(x, y);
    let lum = u16::from(pixel[0]) + u16::from(pixel[1]) + u16::from(pixel[2]);
    let ink = if lum > 384 { 28.0 } else { 235.0 };
    let mix = if strong { 0.55 } else { 0.28 };
    let blend = |channel: u8| (f32::from(channel) * (1.0 - mix) + ink * mix) as u8;
    img.put_pixel(
        x,
        y,
        Rgba([blend(pixel[0]), blend(pixel[1]), blend(pixel[2]), 255]),
    );
}

fn labels(img: &RgbaImage, transform: &Transform, out: &mut Vec<(i32, i32)>) {
    let width = img.width() as i32 - MARGIN_LEFT as i32 - MARGIN_RIGHT as i32;
    let height = img.height() as i32 - MARGIN_TOP as i32 - MARGIN_BOTTOM as i32;
    let mut x = 0;
    while x < width {
        if x % 100 == 0 {
            let label = screen_tick(
                transform.desktop.x,
                transform.desktop.width,
                transform.width,
                x,
            )
            .to_string();
            queue_text(&label, MARGIN_LEFT as i32 + x + 2, 4, out);
        }
        x += 100;
    }
    let mut y = 0;
    while y < height {
        if y % 100 == 0 {
            let label = screen_tick(
                transform.desktop.y,
                transform.desktop.height,
                transform.height,
                y,
            )
            .to_string();
            let label_w = text_width(&label);
            let label_x = MARGIN_LEFT as i32 - label_w - 6;
            let label_y = MARGIN_TOP as i32 + y + 2;
            if label_x >= 2 {
                queue_text(&label, label_x, label_y, out);
            }
        }
        y += 100;
    }
}

pub(crate) fn screen_tick(origin: f64, span: f64, pixels: u32, at: i32) -> i64 {
    (origin + f64::from(at) * span / f64::from(pixels)).round() as i64
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
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
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
        assert_eq!(
            (image.width(), image.height()),
            (320 + MARGIN_LEFT + 8, 200 + MARGIN_TOP + 8)
        );
        let page = |x: u32, y: u32| image.get_pixel(MARGIN_LEFT + x, MARGIN_TOP + y).0;
        assert_ne!(
            page(100, 100),
            [20, 40, 60, 255],
            "major grid crosses the page"
        );
        assert_eq!(
            page(160, 130),
            [20, 40, 60, 255],
            "digits stay off the page"
        );
        assert_eq!(image.get_pixel(2, 2).0, [24, 24, 24, 255]);
        let caption = "x2.000 y2.500";
        let origin = image.width() as i32 - text_width(caption) - 4;
        let mut marked = false;
        for y in 4..18 {
            for x in origin..origin + 10 {
                if x >= 0 && image.get_pixel(x as u32, y as u32).0[0] == 255 {
                    marked = true;
                }
            }
        }
        assert!(marked, "scale caption stays in the margin");
    }

    #[test]
    fn window_ticks_use_the_screen_position() {
        assert_eq!(screen_tick(200.0, 1000.0, 1000, 0), 200);
        assert_eq!(screen_tick(200.0, 1000.0, 1000, 680), 880);
        assert_eq!(screen_tick(80.0, 800.0, 800, 16), 96);
        assert_eq!(screen_tick(-1920.0, 1920.0, 960, 100), -1720);
        assert_eq!(screen_tick(0.0, 1920.0, 1920, 0), 0);
    }

    #[test]
    fn small_frames_are_left_alone() {
        let (png, transform) = frame(80, 40, 80.0, 40.0);
        assert!(stamp(&png, &transform).is_none());
    }
}
