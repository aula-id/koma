//! The trajectory projection: the room's map, drawn from the shelf.
//!
//! The shelf is the source of truth. A lane may publish its own graph of what
//! it is exploring; this merges that with what the shelf proves, so a claim
//! reads as verified only when a finding exists for it.
//!
//! - [`gate`]: open, verified or returned, from shelf rows
//! - [`graph`]: merge the lanes' graph with the shelf
//! - [`layout`]: columns by link depth, rows by lane
//! - [`view`]: the projection a UI or the CLI draws
//! - [`render`]: outline tree, layered graph, dot, mermaid, json
//!
//! The swim-lane view needs the lanes' activity history rather than the shelf,
//! so it lives with the UI.

pub mod gate;
pub mod graph;
pub mod layout;
pub mod render;
pub mod view;

#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use view::{build, project, TrajectoryView, ViewNode};

/// An output format for `koma room trajectory`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Outline tree; the layered graph when `--wide` fits.
    Ascii,
    Json,
    Dot,
    Mermaid,
}

impl Format {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ascii" | "text" => Some(Format::Ascii),
            "json" => Some(Format::Json),
            "dot" => Some(Format::Dot),
            "mermaid" => Some(Format::Mermaid),
            _ => None,
        }
    }
}

/// Render views in a format. `Ascii` is the outline tree; with `layered_width`
/// set it tries the layered graph first and falls back to the outline when the
/// graph is too wide.
pub fn render_all(views: &[TrajectoryView], format: Format, layered_width: Option<usize>) -> Result<String, String> {
    match format {
        Format::Json => render::json(views),
        Format::Dot => Ok(views.iter().map(render::dot).collect::<Vec<_>>().join("\n")),
        Format::Mermaid => Ok(views.iter().map(render::mermaid).collect::<Vec<_>>().join("\n")),
        Format::Ascii => Ok(views
            .iter()
            .map(|v| layered_width.and_then(|w| render::layered(v, w)).unwrap_or_else(|| render::outline(v)))
            .collect::<Vec<_>>()
            .join("\n")),
    }
}
