//! Enums and constants for the `/mcp` dashboard sub-mode state machine and
//! field navigation.
//!
//! Mirrors the `/agents` types module but simpler: MCP servers persist in
//! `config.json` (no markdown files, no pickers), so the only moving parts are
//! the sub-mode state machine and a transport-conditional field list.

use crate::model::app_config::McpTransport;

/// The active sub-mode of the `/mcp` dashboard.
///
/// ```text
///   Browse ── →/Enter ──▶ Edit ── s ──▶ save ──▶ Browse
///     │                     │
///     │── n ──▶ Create ── s ──▶ create ──▶ Browse
///     │
///     └── d ──▶ DeleteConfirm ── y ──▶ delete ──▶ Browse
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpSubMode {
    /// Navigating the server list / reading the selected server (read-only).
    Browse,
    /// Editing an existing server's fields.
    Edit,
    /// Creating a new server (same field editor, fresh drafts + minted uuid).
    Create,
    /// Confirming deletion of the selected server (`y`/`n`).
    DeleteConfirm,
}

/// One editable field in the Edit/Create detail editor, in display/nav order.
///
/// The visible set is transport-conditional: `Command`/`Args`/`Env` only show
/// for [`McpTransport::Stdio`]; `Url` only shows for [`McpTransport::Http`].
/// `Name`/`Enabled`/`Transport` are always present. Compute the live list with
/// [`super::McpState::fields`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpEditField {
    /// Human-facing server name (also the `mcp__<name>__<tool>` namespace source).
    Name,
    /// Whether the server is connected at startup (bool toggle).
    Enabled,
    /// Wire transport (Stdio <-> Http enum toggle). Changing it swaps which of the
    /// transport-specific fields below are visible.
    Transport,
    /// Stdio only: the executable to launch (e.g. `npx`).
    Command,
    /// Stdio only: whitespace-separated arguments (stored as `Vec<String>`).
    Args,
    /// Stdio only: `KEY=VAL, KEY2=VAL2` environment pairs (stored as `Vec<(String,String)>`).
    Env,
    /// Http only: the streamable-HTTP MCP endpoint URL.
    Url,
}

impl McpEditField {
    /// Left-column label for the detail editor.
    pub fn label(self) -> &'static str {
        match self {
            McpEditField::Name => "name",
            McpEditField::Enabled => "enabled",
            McpEditField::Transport => "transport",
            McpEditField::Command => "command",
            McpEditField::Args => "args",
            McpEditField::Env => "env",
            McpEditField::Url => "url",
        }
    }

    /// `true` for the two non-text fields (toggled with ←/→, never typed into):
    /// the Enabled bool and the Transport enum.
    pub fn is_toggle(self) -> bool {
        matches!(self, McpEditField::Enabled | McpEditField::Transport)
    }
}

/// Always-present leading fields (name, enabled, transport).
pub(super) const COMMON_FIELDS: &[McpEditField] = &[
    McpEditField::Name,
    McpEditField::Enabled,
    McpEditField::Transport,
];

/// Stdio-only trailing fields (command, args, env).
pub(super) const STDIO_FIELDS: &[McpEditField] =
    &[McpEditField::Command, McpEditField::Args, McpEditField::Env];

/// Http-only trailing fields (url).
pub(super) const HTTP_FIELDS: &[McpEditField] = &[McpEditField::Url];

/// The full visible field list for a given `transport`, in nav order.
pub(super) fn fields_for(transport: McpTransport) -> Vec<McpEditField> {
    let mut v = COMMON_FIELDS.to_vec();
    match transport {
        McpTransport::Stdio => v.extend_from_slice(STDIO_FIELDS),
        McpTransport::Http => v.extend_from_slice(HTTP_FIELDS),
    }
    v
}
