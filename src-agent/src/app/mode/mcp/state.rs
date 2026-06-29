//! [`McpState`] — the working state for the in-app `/mcp` dashboard — and its
//! associated helpers.
//!
//! Unlike `/agents` (markdown files), MCP servers live in `config.json`. The
//! dashboard holds a CLONE of `config.mcp_servers` plus the per-field drafts;
//! create/edit/delete mutate the clone and the caller persists the real config.
//! Args/env are edited as raw text and parsed to/from their persisted Vec forms.

use crate::model::app_config::{new_uuid, McpServerEntry, McpTransport};

use super::types::{fields_for, McpEditField, McpSubMode};

/// Working state for the in-app `/mcp` dashboard.
///
/// `servers` is a snapshot clone of `config.mcp_servers`; the runtime writes the
/// committed change back into the real config (and calls `save()`) on create /
/// edit / delete, then refreshes this snapshot from disk-config.
#[derive(Debug, Clone)]
pub struct McpState {
    /// Snapshot of the configured MCP servers (clone of `config.mcp_servers`).
    pub servers: Vec<McpServerEntry>,
    /// Selected index into `servers` (the LIST cursor).
    pub list_sel: usize,
    /// `false` = focus on the LIST pane; `true` = focus on the DETAIL pane.
    pub in_detail: bool,
    /// Active sub-mode (Browse / Edit / Create / DeleteConfirm).
    pub mode: McpSubMode,
    /// Highlighted field within the Edit/Create editor.
    pub field: McpEditField,
    /// `true` while typing into the highlighted text field; `false` = navigating.
    pub editing: bool,

    // --- Drafts (seeded on enter_edit / enter_create, read back on save) ---
    /// Draft: uuid. Minted fresh on Create; copied from the entry on Edit so the
    /// save can find the entry to overwrite (and the tool namespace stays stable).
    pub draft_uuid: String,
    /// Draft: server name.
    pub draft_name: String,
    /// Draft: enabled flag (toggled with space/enter or ←/→).
    pub draft_enabled: bool,
    /// Draft: wire transport (toggled with space/enter or ←/→).
    pub draft_transport: McpTransport,
    /// Draft (Stdio): the executable to launch.
    pub draft_command: String,
    /// Draft (Stdio): arguments as raw whitespace-separated text (split on save).
    pub draft_args: String,
    /// Draft (Stdio): env as raw `KEY=VAL, KEY2=VAL2` text (parsed on save).
    pub draft_env: String,
    /// Draft (Http): the endpoint URL.
    pub draft_url: String,

    /// LIVE per-server tool counts (server uuid -> tool count) when this state was
    /// rebuilt on a thin client from a wire snapshot — the client owns no MCP
    /// manager, so the view falls back to this projected map for its status column.
    /// `None` for a local-TUI instance (which reads status from the live manager).
    pub shadow_status: Option<std::collections::HashMap<String, usize>>,
}

impl McpState {
    /// Build the dashboard from the current config's server list: clone it and
    /// start in Browse with the LIST focused.
    pub fn from(servers: &[McpServerEntry]) -> Self {
        Self {
            servers: servers.to_vec(),
            list_sel: 0,
            in_detail: false,
            mode: McpSubMode::Browse,
            field: McpEditField::Name,
            editing: false,
            draft_uuid: String::new(),
            draft_name: String::new(),
            draft_enabled: true,
            draft_transport: McpTransport::Stdio,
            draft_command: String::new(),
            draft_args: String::new(),
            draft_env: String::new(),
            draft_url: String::new(),
            // Local-TUI instance: status comes from the live manager, not a snapshot.
            shadow_status: None,
        }
    }

    /// Replace the snapshot from a fresh config server list and clamp the cursor.
    ///
    /// Called after every create/edit/delete so the LIST reflects saved state.
    pub fn reload(&mut self, servers: &[McpServerEntry]) {
        self.servers = servers.to_vec();
        if self.list_sel >= self.servers.len() {
            self.list_sel = self.servers.len().saturating_sub(1);
        }
    }

    /// The currently-selected server, if any.
    pub fn current(&self) -> Option<&McpServerEntry> {
        self.servers.get(self.list_sel)
    }

    /// The fields visible in the current editor (depends on the draft transport).
    pub fn fields(&self) -> Vec<McpEditField> {
        fields_for(self.draft_transport)
    }

    // --- LIST navigation (Browse) ---

    /// Move the LIST cursor up.
    pub fn list_up(&mut self) {
        self.list_sel = self.list_sel.saturating_sub(1);
    }

    /// Move the LIST cursor down.
    pub fn list_down(&mut self) {
        if self.list_sel + 1 < self.servers.len() {
            self.list_sel += 1;
        }
    }

    // --- Editor field navigation (Edit / Create) ---

    /// Move the editor cursor to the previous field.
    pub fn field_up(&mut self) {
        let fields = self.fields();
        let cur = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        let next = cur.saturating_sub(1);
        self.field = fields[next];
    }

    /// Move the editor cursor to the next field.
    pub fn field_down(&mut self) {
        let fields = self.fields();
        let cur = fields.iter().position(|f| *f == self.field).unwrap_or(0);
        if cur + 1 < fields.len() {
            self.field = fields[cur + 1];
        }
    }

    /// Mutable handle to the draft buffer for the highlighted TEXT field.
    ///
    /// The two toggle fields (Enabled / Transport) have no text buffer; the input
    /// handler toggles them instead of setting `editing`, so this is never called
    /// while one of them is highlighted.
    fn draft_mut(&mut self) -> &mut String {
        match self.field {
            McpEditField::Name => &mut self.draft_name,
            McpEditField::Command => &mut self.draft_command,
            McpEditField::Args => &mut self.draft_args,
            McpEditField::Env => &mut self.draft_env,
            McpEditField::Url => &mut self.draft_url,
            // Toggles are never edited as text.
            McpEditField::Enabled | McpEditField::Transport => {
                unreachable!("toggle fields are not text-edited")
            }
        }
    }

    /// Immutable handle to the draft buffer for text field `f` (view-side reads).
    /// Toggle fields render from their own draft bool/enum, so they return `""`.
    pub fn draft(&self, f: McpEditField) -> &str {
        match f {
            McpEditField::Name => &self.draft_name,
            McpEditField::Command => &self.draft_command,
            McpEditField::Args => &self.draft_args,
            McpEditField::Env => &self.draft_env,
            McpEditField::Url => &self.draft_url,
            McpEditField::Enabled | McpEditField::Transport => "",
        }
    }

    /// Append `c` to the active text draft (no-op on toggle fields).
    pub fn push_char(&mut self, c: char) {
        if self.field.is_toggle() {
            return;
        }
        self.draft_mut().push(c);
    }

    /// Delete the last char of the active text draft (no-op on toggle fields).
    pub fn backspace(&mut self) {
        if self.field.is_toggle() {
            return;
        }
        self.draft_mut().pop();
    }

    /// Toggle the highlighted field's value: Enabled flips bool; Transport flips
    /// Stdio<->Http (and re-points `field` to a still-visible field afterwards).
    pub fn toggle_field(&mut self) {
        match self.field {
            McpEditField::Enabled => self.draft_enabled = !self.draft_enabled,
            McpEditField::Transport => {
                self.draft_transport = match self.draft_transport {
                    McpTransport::Stdio => McpTransport::Http,
                    McpTransport::Http => McpTransport::Stdio,
                };
                // The visible field set just changed; Transport itself is always
                // present, so keeping `field = Transport` is safe. (No clamp needed
                // because we toggle while standing on the Transport row.)
            }
            _ => {}
        }
    }

    // --- Sub-mode transitions ---

    /// Enter EDIT for the selected server, seeding drafts from it.
    pub fn enter_edit(&mut self) {
        let Some(s) = self.current().cloned() else {
            return;
        };
        self.draft_uuid = s.uuid.clone();
        self.draft_name = s.name.clone();
        self.draft_enabled = s.enabled;
        self.draft_transport = s.transport;
        self.draft_command = s.command.clone();
        self.draft_args = s.args.join(" ");
        self.draft_env = join_env(&s.env);
        self.draft_url = s.url.clone();
        self.mode = McpSubMode::Edit;
        self.field = McpEditField::Name;
        self.in_detail = true;
        self.editing = false;
    }

    /// Enter CREATE: reset every draft, mint a fresh uuid, focus the name.
    pub fn enter_create(&mut self) {
        self.draft_uuid = new_uuid();
        self.draft_name = String::new();
        self.draft_enabled = true;
        self.draft_transport = McpTransport::Stdio;
        self.draft_command = String::new();
        self.draft_args = String::new();
        self.draft_env = String::new();
        self.draft_url = String::new();
        self.mode = McpSubMode::Create;
        self.field = McpEditField::Name;
        self.in_detail = true;
        self.editing = false;
    }

    /// Enter DELETE-CONFIRM for the selected server.
    pub fn enter_delete(&mut self) {
        self.mode = McpSubMode::DeleteConfirm;
        self.editing = false;
    }

    /// Discard drafts and return to Browse with the LIST focused.
    pub fn cancel(&mut self) {
        self.mode = McpSubMode::Browse;
        self.editing = false;
        self.in_detail = false;
        self.field = McpEditField::Name;
    }

    /// Build an [`McpServerEntry`] from the current drafts (the value the runtime
    /// writes into `config.mcp_servers` on create/save).
    ///
    /// Args split on whitespace (empties dropped); env parsed from
    /// `KEY=VAL, KEY2=VAL2`. The transport-irrelevant fields are still written
    /// (e.g. `url` on a Stdio server) but stay empty because they were never
    /// shown/edited — exactly the union-of-fields shape `McpServerEntry` expects.
    pub fn to_entry(&self) -> McpServerEntry {
        McpServerEntry {
            uuid: self.draft_uuid.clone(),
            name: self.draft_name.trim().to_string(),
            enabled: self.draft_enabled,
            transport: self.draft_transport,
            command: self.draft_command.trim().to_string(),
            args: parse_args(&self.draft_args),
            env: parse_env(&self.draft_env),
            url: self.draft_url.trim().to_string(),
        }
    }
}

/// Split raw args text into a `Vec<String>` on any whitespace, dropping empties.
pub(super) fn parse_args(raw: &str) -> Vec<String> {
    raw.split_whitespace().map(|s| s.to_string()).collect()
}

/// Parse `KEY=VAL, KEY2=VAL2` env text into ordered `(key, value)` pairs.
///
/// Entries are comma-separated; each is split on the FIRST `=` (so values may
/// contain `=`). Blank entries and entries without a non-empty key are dropped.
/// The value is taken verbatim after the `=` (only outer whitespace trimmed), so
/// a `KEY=` yields an empty value.
pub(super) fn parse_env(raw: &str) -> Vec<(String, String)> {
    raw.split(',')
        .filter_map(|pair| {
            let pair = pair.trim();
            if pair.is_empty() {
                return None;
            }
            let (k, v) = match pair.split_once('=') {
                Some((k, v)) => (k.trim().to_string(), v.trim().to_string()),
                // No `=` → treat the whole token as a bare key with an empty value.
                None => (pair.to_string(), String::new()),
            };
            if k.is_empty() {
                None
            } else {
                Some((k, v))
            }
        })
        .collect()
}

/// Render `(key, value)` env pairs back into the `KEY=VAL, KEY2=VAL2` edit form
/// (the inverse of [`parse_env`], used to seed the draft on Edit).
pub(super) fn join_env(env: &[(String, String)]) -> String {
    env.iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Short label for a transport, shown in the LIST + detail rows.
pub fn transport_label(t: McpTransport) -> &'static str {
    match t {
        McpTransport::Stdio => "stdio",
        McpTransport::Http => "http",
    }
}
