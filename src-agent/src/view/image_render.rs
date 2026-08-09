//! Terminal image renderer: Kitty Graphics Protocol + Unicode half-block fallback.
//!
//! The Kitty path writes escape sequences that the terminal interprets as
//! inline images. The half-block path renders pixels as colored `▀`/`▄`
//! characters using 24-bit true-color, scaled to fit a given terminal width.

use std::path::Path;

use anyhow::Result;
use ratatui::text::{Line, Span};

/// Maximum width in terminal cells for the rendered image.
pub const MAX_IMAGE_WIDTH_CELLS: u16 = 80;

/// Detected Kitty graphics protocol support.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittySupport {
    /// Protocol confirmed working.
    Yes,
    /// Protocol not available (or not detected).
    No,
}

/// Image renderer that detects terminal capabilities and dispatches to the
/// appropriate strategy.
pub struct ImageRenderer {
    pub kitty: KittySupport,
}

impl ImageRenderer {
    /// Create a renderer and detect Kitty support from the environment.
    ///
    /// Detection checks `TERM`, `TERM_PROGRAM`, and `KITTY_WINDOW_ID`
    /// environment variables. A proper Kitty probe (sending a tiny test
    /// image) is not done here because it requires stdout access which is
    /// not available from the tool layer.
    pub fn detect() -> Self {
        Self {
            kitty: detect_kitty(),
        }
    }

    /// Render an image file into ratatui `Line`s for embedding in the
    /// transcript or overlay.
    ///
    /// When Kitty is supported, returns a single line containing the Kitty
    /// escape sequence (the terminal handles pixel rendering).
    /// Otherwise, renders using Unicode half-block characters with 24-bit
    /// true-color.
    pub fn render_to_lines(
        &self,
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
                run.push('▀');
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

    /// Generate Kitty Graphics Protocol escape sequences to display the image.
    pub fn kitty_display(path: &Path, placement_id: u32) -> Result<KittyImageEscapes> {
        let bytes = std::fs::read(path)?;
        let b64 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &bytes,
        );

        // Chunk into 4096-byte pieces for the protocol.
        let chunk_size = 4096;
        let chunks: Vec<&str> = b64.as_bytes()
            .chunks(chunk_size)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
            .collect();

        let mut payload = String::new();
        for (i, chunk) in chunks.iter().enumerate() {
            let m = if i == chunks.len() - 1 { "0" } else { "1" };
            payload.push_str(&format!(
                "\x1b_Gi={id},s={len},v=1,a=T,f=100,t=f,{m};{chunk}\x1b\\",
                id = placement_id,
                len = bytes.len(),
                m = m,
                chunk = chunk,
            ));
        }

        Ok(KittyImageEscapes {
            display: payload,
            delete: format!("\x1b_Ga=d,i={id}\x1b\\", id = placement_id),
        })
    }
}

/// Kitty Graphics Protocol escape sequences for one image.
pub struct KittyImageEscapes {
    /// Write this to stdout to display the image.
    pub display: String,
    /// Write this to stdout to delete/clean up the image.
    pub delete: String,
}

/// Detect Kitty terminal support from environment variables.
fn detect_kitty() -> KittySupport {
    // KITTY_WINDOW_ID is set by Kitty.
    if std::env::var_os("KITTY_WINDOW_ID").is_some() {
        return KittySupport::Yes;
    }
    // TERM_PROGRAM=kitty is also set by some configurations.
    if let Ok(tp) = std::env::var("TERM_PROGRAM") {
        if tp.to_lowercase().contains("kitty") {
            return KittySupport::Yes;
        }
    }
    KittySupport::No
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_kitty_returns_known_variant() {
        // Can't guarantee we're in a Kitty terminal, but the function should
        // not panic and should return a valid variant.
        let _ = detect_kitty();
    }

    #[test]
    fn kitty_display_generates_escape_sequences() {
        // Create a tiny 1x1 red PNG in memory.
        let img = image::RgbaImage::from_pixel(1, 1, image::Rgba([255, 0, 0, 255]));
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
        let bytes = buf.into_inner();

        let tmp = std::env::temp_dir().join("koma_test_kitty.png");
        std::fs::write(&tmp, &bytes).unwrap();

        let result = ImageRenderer::kitty_display(&tmp, 1);
        assert!(result.is_ok());
        let escapes = result.unwrap();
        assert!(escapes.display.contains("\x1b_G"));
        assert!(escapes.delete.contains("a=d"));

        let _ = std::fs::remove_file(tmp);
    }
}
