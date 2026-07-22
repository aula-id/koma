//! Colour palette resolution for the TUI.
//!
//! A [`Palette`] is derived from the global [`AppConfig`] on every `draw` call
//! (it's a plain struct copy — no heap allocation). All views receive a
//! `&Palette` so every colour in the UI flows through one place.
//!
//! Accent names are validated against [`ACCENTS`]; unknown strings fall back to
//! the green mapping so a typo in `config.json` never breaks the UI.

use crate::model::app_config::AppConfig;
use ratatui::style::Color;

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
pub const ACCENTS: &[&str] = &[
    "green", "cyan", "blue", "magenta", "yellow", "red", "white", "orange", "pink",
];

/// Resolve an accent name + theme into a concrete [`Color`].
///
/// Exposed crate-wide so the settings view can colour the accent name in its
/// own resolved tint without duplicating the mapping.
pub(crate) fn resolve_accent(name: &str, dark: bool) -> Color {
    match (name, dark) {
        ("green", true) => Color::Rgb(57, 255, 20),
        ("green", false) => Color::Rgb(0, 128, 0),
        ("cyan", true) => Color::Rgb(0, 255, 255),
        ("cyan", false) => Color::Rgb(0, 128, 128),
        ("blue", true) => Color::Rgb(90, 160, 255),
        ("blue", false) => Color::Rgb(0, 0, 200),
        ("magenta", true) => Color::Rgb(255, 90, 255),
        ("magenta", false) => Color::Rgb(160, 0, 160),
        ("yellow", true) => Color::Rgb(255, 225, 60),
        ("yellow", false) => Color::Rgb(160, 120, 0),
        ("red", true) => Color::Rgb(255, 90, 90),
        ("red", false) => Color::Rgb(200, 0, 0),
        ("white", true) => Color::White,
        ("white", false) => Color::Rgb(20, 20, 20),
        ("orange", true) => Color::Rgb(255, 140, 0),
        ("orange", false) => Color::Rgb(200, 100, 0),
        ("pink", true) => Color::Rgb(255, 105, 180),
        ("pink", false) => Color::Rgb(200, 60, 120),
        // Unknown accent string → fall back to the green mapping for the theme.
        (_, true) => Color::Rgb(57, 255, 20),
        (_, false) => Color::Rgb(0, 128, 0),
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

/// Build the FOREST palette — a dark earthy canvas with warm autumn tones.
pub fn forest() -> Palette {
    Palette {
        bg: Color::Rgb(51, 61, 41),
        fg: Color::Rgb(194, 197, 170),
        dim: Color::Rgb(164, 172, 134),
        accent: Color::Rgb(166, 138, 100),
        panel: Color::Rgb(65, 72, 51),
        sel_fg: Color::Rgb(51, 61, 41),
        sel_bg: Color::Rgb(166, 138, 100),
        success: Color::Rgb(101, 109, 74),
        warn: Color::Rgb(147, 102, 57),
        error: Color::Rgb(176, 74, 52),
        info: Color::Rgb(122, 150, 130),
        heat: [
            Color::Rgb(65, 72, 51),
            Color::Rgb(101, 109, 74),
            Color::Rgb(166, 138, 100),
            Color::Rgb(147, 102, 57),
            Color::Rgb(176, 74, 52),
        ],
    }
}

/// Build the AUTUMN palette — a warm dark canvas with burnt orange and amber tones.
pub fn autumn() -> Palette {
    Palette {
        bg: Color::Rgb(46, 42, 32),
        fg: Color::Rgb(241, 220, 167),
        dim: Color::Rgb(186, 165, 135),
        accent: Color::Rgb(255, 203, 105),
        panel: Color::Rgb(61, 55, 41),
        sel_fg: Color::Rgb(46, 42, 32),
        sel_bg: Color::Rgb(255, 203, 105),
        success: Color::Rgb(155, 155, 122),
        warn: Color::Rgb(232, 172, 101),
        error: Color::Rgb(200, 90, 60),
        info: Color::Rgb(125, 155, 134),
        heat: [
            Color::Rgb(61, 55, 41),
            Color::Rgb(121, 125, 98),
            Color::Rgb(186, 165, 135),
            Color::Rgb(232, 172, 101),
            Color::Rgb(200, 90, 60),
        ],
    }
}

/// Build the WARM palette — a light peachy canvas with coral accents.
pub fn warm() -> Palette {
    Palette {
        bg: Color::Rgb(255, 228, 213),
        fg: Color::Rgb(91, 58, 48),
        dim: Color::Rgb(150, 102, 86),
        accent: Color::Rgb(224, 96, 63),
        panel: Color::Rgb(251, 212, 180),
        sel_fg: Color::Rgb(255, 228, 213),
        sel_bg: Color::Rgb(224, 96, 63),
        success: Color::Rgb(95, 140, 66),
        warn: Color::Rgb(200, 120, 20),
        error: Color::Rgb(198, 50, 45),
        info: Color::Rgb(46, 120, 168),
        heat: [
            Color::Rgb(253, 220, 197),
            Color::Rgb(246, 196, 146),
            Color::Rgb(249, 177, 110),
            Color::Rgb(248, 161, 116),
            Color::Rgb(247, 144, 122),
        ],
    }
}

/// Build the COLD SYMPHONY palette — a light cool blue canvas with smooth blue accents.
pub fn cold_symphony() -> Palette {
    Palette {
        bg: Color::Rgb(226, 234, 252),
        fg: Color::Rgb(30, 52, 92),
        dim: Color::Rgb(95, 120, 165),
        accent: Color::Rgb(74, 124, 224),
        panel: Color::Rgb(192, 211, 249),
        sel_fg: Color::Rgb(226, 234, 252),
        sel_bg: Color::Rgb(74, 124, 224),
        success: Color::Rgb(60, 150, 110),
        warn: Color::Rgb(200, 140, 40),
        error: Color::Rgb(205, 70, 70),
        info: Color::Rgb(106, 153, 241),
        heat: [
            Color::Rgb(218, 255, 239),
            Color::Rgb(169, 240, 229),
            Color::Rgb(120, 224, 219),
            Color::Rgb(136, 175, 244),
            Color::Rgb(106, 153, 241),
        ],
    }
}

/// Build the WINTER palette — a light frosty canvas with cool pink accents.
pub fn winter() -> Palette {
    Palette {
        bg: Color::Rgb(239, 247, 246),
        fg: Color::Rgb(42, 58, 68),
        dim: Color::Rgb(110, 135, 145),
        accent: Color::Rgb(214, 110, 164),
        panel: Color::Rgb(209, 247, 243),
        sel_fg: Color::Rgb(239, 247, 246),
        sel_bg: Color::Rgb(214, 110, 164),
        success: Color::Rgb(70, 160, 120),
        warn: Color::Rgb(200, 140, 40),
        error: Color::Rgb(205, 80, 90),
        info: Color::Rgb(70, 175, 205),
        heat: [
            Color::Rgb(209, 247, 243),
            Color::Rgb(178, 247, 239),
            Color::Rgb(123, 223, 242),
            Color::Rgb(245, 198, 218),
            Color::Rgb(242, 181, 212),
        ],
    }
}

/// Build the MONOKAI palette — a classic dark theme with vibrant neon accents.
pub fn monokai() -> Palette {
    Palette {
        bg: Color::Rgb(39, 40, 34),
        fg: Color::Rgb(248, 248, 242),
        dim: Color::Rgb(117, 113, 94),
        accent: Color::Rgb(249, 38, 114),
        panel: Color::Rgb(62, 61, 50),
        sel_fg: Color::Rgb(248, 248, 242),
        sel_bg: Color::Rgb(73, 72, 62),
        success: Color::Rgb(166, 226, 46),
        warn: Color::Rgb(253, 151, 31),
        error: Color::Rgb(249, 38, 114),
        info: Color::Rgb(102, 217, 239),
        heat: [
            Color::Rgb(73, 72, 62),
            Color::Rgb(166, 226, 46),
            Color::Rgb(230, 219, 116),
            Color::Rgb(253, 151, 31),
            Color::Rgb(249, 38, 114),
        ],
    }
}

/// Build the VSCODE palette — Microsoft's Visual Studio Code dark theme.
pub fn vscode() -> Palette {
    Palette {
        bg: Color::Rgb(30, 30, 30),
        fg: Color::Rgb(212, 212, 212),
        dim: Color::Rgb(128, 128, 128),
        accent: Color::Rgb(86, 156, 214),
        panel: Color::Rgb(37, 37, 38),
        sel_fg: Color::Rgb(212, 212, 212),
        sel_bg: Color::Rgb(38, 79, 120),
        success: Color::Rgb(106, 153, 85),
        warn: Color::Rgb(220, 220, 170),
        error: Color::Rgb(244, 71, 71),
        info: Color::Rgb(79, 193, 255),
        heat: [
            Color::Rgb(58, 61, 65),
            Color::Rgb(106, 153, 85),
            Color::Rgb(220, 220, 170),
            Color::Rgb(206, 145, 120),
            Color::Rgb(244, 71, 71),
        ],
    }
}

/// Build the GITHUB DARK palette — GitHub's dark theme with cool blue accents.
pub fn github_dark() -> Palette {
    Palette {
        bg: Color::Rgb(13, 17, 23),
        fg: Color::Rgb(201, 209, 217),
        dim: Color::Rgb(139, 148, 158),
        accent: Color::Rgb(88, 166, 255),
        panel: Color::Rgb(22, 27, 34),
        sel_fg: Color::Rgb(201, 209, 217),
        sel_bg: Color::Rgb(22, 51, 86),
        success: Color::Rgb(63, 185, 80),
        warn: Color::Rgb(210, 153, 34),
        error: Color::Rgb(248, 81, 73),
        info: Color::Rgb(121, 192, 255),
        heat: [
            Color::Rgb(22, 27, 34),
            Color::Rgb(63, 185, 80),
            Color::Rgb(210, 153, 34),
            Color::Rgb(219, 109, 40),
            Color::Rgb(248, 81, 73),
        ],
    }
}

/// A palette constructor — one entry in [`PALETTES`], keyed by its config name.
type PaletteFn = fn() -> Palette;

/// Registry of named palettes. Add a palette = add one line here + its constructor.
pub const PALETTES: &[(&str, PaletteFn)] = &[
    ("dark", dark),
    ("light", light),
    ("forest", forest),
    ("autumn", autumn),
    ("warm", warm),
    ("cold symphony", cold_symphony),
    ("winter", winter),
    ("monokai", monokai),
    ("vscode", vscode),
    ("github dark", github_dark),
];

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
