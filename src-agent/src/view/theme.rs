//! Colour palette resolution for the TUI.
//!
//! A [`Palette`] is derived from the global [`AppConfig`] on every `draw` call
//! (it's a plain struct copy — no heap allocation). All views receive a
//! `&Palette` so every colour in the UI flows through one place.
//!
//! Accent names are validated against [`ACCENTS`]; unknown strings fall back to
//! the green mapping so a typo in `config.json` never breaks the UI.

use ratatui::style::Color;
use crate::model::app_config::AppConfig;

/// All colour roles used by the views.
///
/// Twelve semantic roles plus a five-stop heat ramp. Every field is a `Color`
/// (and `[Color; 5]` is itself `Copy + PartialEq`), so a `Palette` stays a plain
/// `Copy` value — passed by reference to sub-draws at zero cost and usable as the
/// transcript-cache key (`transcript.rs`).
#[derive(Clone, Copy, PartialEq)]
pub struct Palette {
    /// Canvas background painted behind every otherwise-unstyled cell.
    pub bg: Color,
    /// Primary text colour.
    pub fg: Color,
    /// Muted text / borders (secondary / status / dim text).
    pub dim: Color,
    /// Highlights: rail, ✓, box labels; also the source for `sel_bg`.
    pub accent: Color,
    /// Raised surface: user message band, boxes/overlays (was `user_band`).
    pub panel: Color,
    /// Foreground on a selected list row (overlaid on `sel_bg`).
    pub sel_fg: Color,
    /// Background for the selected list row.
    pub sel_bg: Color,
    /// Green success cues.
    pub success: Color,
    /// Amber warning cues.
    pub warn: Color,
    /// Red error cues.
    pub error: Color,
    /// Blue info cues (Plan badge, shimmer base).
    pub info: Color,
    /// Usage-heatmap ramp, coldest (empty) → hottest.
    pub heat: [Color; 5],
}

/// The ordered list of valid accent names exposed to users and the `/settings`
/// UI. Unknown strings in `config.json` fall back to "green".
///
/// Consumed by the `/settings` dashboard to cycle the accent draft.
pub const ACCENTS: &[&str] = &["green", "cyan", "blue", "magenta", "yellow", "red", "white", "orange", "pink"];

/// Resolve an accent name + theme into a concrete [`Color`].
///
/// Exposed crate-wide so the settings view can colour the accent name in its
/// own resolved tint without duplicating the mapping.
pub(crate) fn resolve_accent(name: &str, dark: bool) -> Color {
    match (name, dark) {
        ("green",   true)  => Color::Rgb(57, 255, 20),
        ("green",   false) => Color::Rgb(0, 128, 0),
        ("cyan",    true)  => Color::Rgb(0, 255, 255),
        ("cyan",    false) => Color::Rgb(0, 128, 128),
        ("blue",    true)  => Color::Rgb(90, 160, 255),
        ("blue",    false) => Color::Rgb(0, 0, 200),
        ("magenta", true)  => Color::Rgb(255, 90, 255),
        ("magenta", false) => Color::Rgb(160, 0, 160),
        ("yellow",  true)  => Color::Rgb(255, 225, 60),
        ("yellow",  false) => Color::Rgb(160, 120, 0),
        ("red",     true)  => Color::Rgb(255, 90, 90),
        ("red",     false) => Color::Rgb(200, 0, 0),
        ("white",   true)  => Color::White,
        ("white",   false) => Color::Rgb(20, 20, 20),
        ("orange",  true)  => Color::Rgb(255, 140, 0),
        ("orange",  false) => Color::Rgb(200, 100, 0),
        ("pink",    true)  => Color::Rgb(255, 105, 180),
        ("pink",    false) => Color::Rgb(200, 60, 120),
        // Unknown accent string → fall back to the green mapping for the theme.
        (_,         true)  => Color::Rgb(57, 255, 20),
        (_,         false) => Color::Rgb(0, 128, 0),
    }
}

/// Build the default DARK palette — today's green-on-black look, expanded with the
/// new semantic roles and heat ramp.
///
/// `accent`/`sel_bg` reuse `resolve_accent("green", true)` so the default look is
/// identical to the pre-refactor theme.
pub fn dark() -> Palette {
    let accent = resolve_accent("green", true);
    Palette {
        bg: Color::Rgb(0, 0, 0),
        fg: Color::Rgb(230, 230, 230),
        dim: Color::Rgb(173, 173, 173),
        accent,
        panel: Color::Rgb(43, 47, 56),
        // Color::Black/White are ANSI palette colors; on BOLD text terminals brighten
        // ANSI black to gray, so the inverse selection text would look gray. True-color
        // RGB bypasses the 16-color palette — the text stays truly black on the accent.
        sel_fg: Color::Rgb(0, 0, 0),
        sel_bg: accent,
        success: Color::Rgb(0, 200, 83),
        warn: Color::Rgb(255, 180, 60),
        error: Color::Rgb(255, 60, 60),
        info: Color::Rgb(80, 200, 255),
        heat: [
            Color::Rgb(35, 35, 35),
            Color::Rgb(0, 120, 60),
            Color::Rgb(100, 160, 50),
            Color::Rgb(200, 140, 0),
            Color::Rgb(220, 50, 50),
        ],
    }
}

/// Build the LIGHT palette — a milk-white canvas with the green accent.
pub fn light() -> Palette {
    let accent = resolve_accent("green", false);
    Palette {
        bg: Color::Rgb(250, 250, 246), // milk white
        fg: Color::Rgb(20, 20, 20),
        dim: Color::Rgb(120, 120, 120),
        accent,
        panel: Color::Rgb(228, 230, 235),
        // RGB (not ANSI) white so BOLD selection text isn't dimmed by the terminal.
        sel_fg: Color::Rgb(255, 255, 255),
        sel_bg: accent,
        success: Color::Rgb(0, 150, 60),
        warn: Color::Rgb(200, 120, 0),
        error: Color::Rgb(200, 40, 40),
        info: Color::Rgb(30, 120, 200),
        heat: [
            Color::Rgb(225, 225, 225),
            Color::Rgb(0, 150, 70),
            Color::Rgb(120, 170, 40),
            Color::Rgb(210, 140, 0),
            Color::Rgb(210, 50, 50),
        ],
    }
}

/// A palette constructor — one entry in [`PALETTES`], keyed by its config name.
type PaletteFn = fn() -> Palette;

/// Registry of named palettes. Add a palette = add one line here + its constructor.
pub const PALETTES: &[(&str, PaletteFn)] = &[("dark", dark), ("light", light)];

/// Build a [`Palette`] by looking up `cfg.palette` in [`PALETTES`], falling back to
/// [`dark`] for an unknown name.
///
/// Called once per frame at the top of `view::draw`. The result is stack-only
/// (all fields are `Copy`) so passing `&palette` to sub-draws is zero cost.
pub fn palette(cfg: &AppConfig) -> Palette {
    PALETTES
        .iter()
        .find(|(name, _)| *name == cfg.palette)
        .map(|(_, build)| build())
        .unwrap_or_else(dark)
}

/// Lighten a color toward white by `t` in [0,1]. Non-Rgb colors pass through.
pub(crate) fn lighten(c: Color, t: f32) -> Color {
    match c {
        Color::Rgb(r, g, b) => {
            let f = t.clamp(0.0, 1.0);
            let lerp = |x: u8| (x as f32 + (255.0 - x as f32) * f).round() as u8;
            Color::Rgb(lerp(r), lerp(g), lerp(b))
        }
        other => other,
    }
}
