//! Terminal image renderer: Unicode half-block fallback.
//!
//! Renders pixels as colored `▀` characters using 24-bit true-color,
//! scaled to fit a given terminal width. Each terminal cell represents
//! 2 vertical pixels via foreground/background colors on `▀`.

use std::path::Path;

use anyhow::Result;
use ratatui::text::{Line, Span};

/// Maximum width in terminal cells for the rendered image.
pub const MAX_IMAGE_WIDTH_CELLS: u16 = 80;

/// Image renderer that produces ratatui `Line`s from an image file.
pub struct ImageRenderer;

impl ImageRenderer {
    /// Render an image file into ratatui `Line`s for embedding in the
    /// chat transcript.
    ///
    /// Uses Unicode half-block characters (`▀`) with 24-bit true-color.
    /// Each cell represents 2 vertical pixels: the top pixel as foreground
    /// and the bottom pixel as background of `▀`.
    pub fn render_to_lines(
        path: &Path,
        max_width_cells: u16,
    ) -> Result<Vec<Line<'static>>> {
        let img = image::open(path)?;
        let rgb_orig = img.to_rgb8();
        let (orig_w, orig_h) = rgb_orig.dimensions();

        // Compute display dimensions in cells.
        // Assume ~8px wide per cell, ~16px tall per cell (typical 10pt monospace).
        // The aspect ratio of a cell is roughly 1:2 (half-block).
        let cell_aspect = 0.5; // width/height of one cell
        let img_aspect = orig_w as f32 / orig_h as f32;

        let display_w = max_width_cells.min(MAX_IMAGE_WIDTH_CELLS);
        let display_h = ((display_w as f32 * cell_aspect) / img_aspect * 2.0) as u16;

        let display_w = display_w.max(1);
        let display_h = display_h.max(1);

        // Resize the image.
        let resized = img.resize_exact(
            display_w as u32 * 2, // 2 pixels per half-block column
            display_h as u32 * 2, // 2 rows per cell (top + bottom half-block)
            image::imageops::FilterType::CatmullRom,
        );

        let rgb = resized.to_rgb8();
        let (rw, rh) = rgb.dimensions();

        // Render using half-block characters.
        let mut lines: Vec<Line<'static>> = Vec::with_capacity(display_h as usize);

        for cell_y in 0..display_h as usize {
            let mut spans: Vec<Span<'static>> = Vec::new();
            let mut current_fg: Option<(u8, u8, u8)> = None;
            let mut current_bg: Option<(u8, u8, u8)> = None;
            let mut run = String::new();

            for cell_x in 0..display_w as usize {
                let px_x = (cell_x * 2).min(rw as usize - 1);
                let px_y_top = (cell_y * 2).min(rh as usize - 1);
                let px_y_bot = (cell_y * 2 + 1).min(rh as usize - 1);

                let top = rgb.get_pixel(px_x as u32, px_y_top as u32);
                let bot = rgb.get_pixel(px_x as u32, px_y_bot as u32);

                let fg = (top[0], top[1], top[2]); // top pixel → foreground (▀)
                let bg = (bot[0], bot[1], bot[2]); // bottom pixel → background

                // Flush the run if colors change.
                if current_fg != Some(fg) || current_bg != Some(bg) {
                    if !run.is_empty() {
                        let (f, b) = (current_fg.unwrap(), current_bg.unwrap());
                        spans.push(Span::styled(
                            std::mem::take(&mut run),
                            ratatui::style::Style::default()
                                .fg(ratatui::style::Color::Rgb(f.0, f.1, f.2))
                                .bg(ratatui::style::Color::Rgb(b.0, b.1, b.2)),
                        ));
                    }
                    current_fg = Some(fg);
                    current_bg = Some(bg);
                }
                run.push('\u{2580}');
            }

            // Flush remaining run.
            if !run.is_empty() {
                let (f, b) = (current_fg.unwrap(), current_bg.unwrap());
                spans.push(Span::styled(
                    run,
                    ratatui::style::Style::default()
                        .fg(ratatui::style::Color::Rgb(f.0, f.1, f.2))
                        .bg(ratatui::style::Color::Rgb(b.0, b.1, b.2)),
                ));
            }

            lines.push(Line::from(spans));
        }

        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
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
}
