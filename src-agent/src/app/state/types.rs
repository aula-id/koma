//! Auxiliary types used by [`super::AppStateRest`] and the rest of the app.
//!
//! - [`AgentMode`]       – tool-approval policy (auto / normal / yolo)
//! - [`ToastKind`]       – visual style of a transient toast box
//! - [`TranscriptCache`] – per-frame rendered-lines cache
//! - [`CataloguePending`] – debounced model-catalogue fetch request

use crate::view::theme::Palette;
use ratatui::text::Line;

/// Tool-approval policy for the agentic loop.
///
/// - `Auto`: every requested tool runs immediately (no prompt) — the original
///   behaviour.
/// - `Normal`: *risky* tools (write/delete) pause the turn for a `y/n` user
///   approval; *safe* tools (read/dir_list/dir_cache_update) still run inline.
/// - `Plan`: a read-only planning/exploration mode — the tool surface is
///   restricted to non-mutating tools (browsing, reasoning) so the model can
///   investigate freely without risking a change. It is exited either by the
///   model submitting a plan for approval, or manually via `/mode` /
///   Shift+Tab; leaving it restores whatever mode was active before entering
///   (see `SessionRuntime::plan_return_mode`). (Read-only tool enforcement and
///   the plan-approval flow land in a later wave — this variant currently
///   only changes the mode label + system-prompt nudge.)
/// - `Yolo`: *risky* tools run inline with NO classifier call and NO `y/n`
///   prompt — the harness is fully bypassed. The deterministic workspace path
///   guard (WC) still applies, so writes stay inside the project. This mode is
///   double-gated: it can only be ENTERED while `yolo_armed` is set (armed from
///   the `/security` panel), so it can never be reached by accident.
///
/// Toggled with Shift+Tab or `/mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AgentMode {
    #[default]
    Auto,
    Normal,
    Plan,
    Yolo,
    Sdlc,
}

impl AgentMode {
    /// Short display label for the header / status line.
    pub fn label(self) -> &'static str {
        match self {
            AgentMode::Auto => "auto",
            AgentMode::Normal => "normal",
            AgentMode::Plan => "plan",
            AgentMode::Yolo => "yolo",
            AgentMode::Sdlc => "sdlc",
        }
    }
    /// Advance to the next mode for the interactive toggle (Shift+Tab / bare
    /// `/mode`), respecting the YOLO arm gate.
    ///
    /// Normal ordering is Auto → Normal → Plan → Sdlc → Auto. Callers that
    /// must keep SDLC locked during execute/integrate (Shift+Tab) gate the
    /// transition themselves using the session phase; this method always
    /// returns the pure next mode in the cycle.
    ///
    /// - `yolo_armed`: retained for call-site compatibility; Yolo folds back
    ///   to Auto either way (Yolo is entered only via explicit `/mode yolo`
    ///   when armed, not via this cycle).
    pub fn cycle(self, _yolo_armed: bool) -> Self {
        match self {
            AgentMode::Auto => AgentMode::Normal,
            AgentMode::Normal => AgentMode::Plan,
            AgentMode::Plan => AgentMode::Sdlc,
            AgentMode::Sdlc => AgentMode::Auto,
            AgentMode::Yolo => AgentMode::Auto,
        }
    }
}

#[cfg(test)]
mod agent_mode_tests {
    use super::AgentMode;

    #[test]
    fn cycle_includes_sdlc_when_unarmed() {
        assert_eq!(AgentMode::Plan.cycle(false), AgentMode::Sdlc);
        // SDLC advances to Auto in the pure cycle; phase gating lives at the
        // Shift+Tab call site.
        assert_eq!(AgentMode::Sdlc.cycle(false), AgentMode::Auto);
    }

    #[test]
    fn cycle_includes_sdlc_when_armed() {
        assert_eq!(AgentMode::Plan.cycle(true), AgentMode::Sdlc);
        assert_eq!(AgentMode::Sdlc.cycle(true), AgentMode::Auto);
        assert_eq!(AgentMode::Yolo.cycle(true), AgentMode::Auto);
    }

    #[test]
    fn label_sdlc() {
        assert_eq!(AgentMode::Sdlc.label(), "sdlc");
    }
}

/// Visual style of the transient toast box.
///
/// - `Error`: red box titled "error" — failures (the original behaviour).
/// - `Info`: neutral accent box titled "info" — non-failure notices (e.g. the
///   post-compaction summary). Rendered multi-line / wrapped, never red so an
///   informational message doesn't read as an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Error,
    Info,
}

/// Per-frame cache of the transcript's rendered visual lines.
///
/// Markdown rendering (pulldown-cmark + syntect highlighting) and span-wrapping
/// are expensive and would otherwise re-run for every committed message on every
/// redraw (every streamed token, every scroll). This caches each NON-system
/// message's fully-rendered visual lines so they are computed once and reused
/// across frames; only NEW messages are rendered. The cache is keyed by the wrap
/// width + palette, so a resize or theme change forces a full rebuild; a shrink
/// of the message list (compaction / resend) also forces a rebuild.
#[derive(Default)]
pub struct TranscriptCache {
    pub width: usize,
    pub palette: Option<Palette>,
    /// One entry per NON-system message, in order; each is that message's
    /// rendered visual lines (bullet+indent applied, no separator).
    pub blocks: Vec<Vec<Line<'static>>>,
}

/// A debounced, pending model-catalogue (`GET {endpoint}/models`) fetch.
///
/// Created/refreshed by [`super::AppStateRest::request_catalogue`] on each omnisearch
/// keystroke or provider change. `due` is pushed ~300 ms into the future every
/// time the same request is re-issued, so a burst of typing collapses into a
/// single fetch fired once the user pauses. The event-loop tick reads `due`; when
/// `now >= due` (and nothing is already in flight) it takes this and spawns the
/// fetch against `endpoint`/`api_key`.
#[derive(Debug, Clone)]
pub struct CataloguePending {
    /// The endpoint to fetch `/models` from.
    pub endpoint: String,
    /// Bearer token for that endpoint (may be empty for a keyless catalogue).
    pub api_key: String,
    /// OAuth uuid backing this fetch (empty = static-key provider, no refresh).
    pub oauth_uuid: String,
    /// Earliest instant the fetch may fire (debounce gate).
    pub due: std::time::Instant,
}
