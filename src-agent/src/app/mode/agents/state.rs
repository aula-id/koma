//! [`AgentsState`] — the working state for the in-app `/agents` dashboard —
//! and its associated helpers.

use std::path::PathBuf;

use crate::model::agent_def::{load_registry, AgentDef, AgentSource};
use crate::model::app_config::AppConfig;
use crate::model::session::Session;
use crate::model::settings::Settings;

use super::picker::{ModelPickerState, ToolPickerState};
use super::types::{
    AgentEditField, AgentScope, AgentSubMode, CREATE_FIELDS, EDIT_FIELDS, TEMPLATE_BODY,
};

/// Working state for the in-app `/agents` dashboard.
///
/// Holds the loaded registry snapshot plus editable drafts. Nothing touches
/// disk until the user confirms a create/edit/delete, at which point the
/// runtime reads these drafts back out and calls the data-layer API.
#[derive(Debug, Clone)]
pub struct AgentsState {
    /// Snapshot of the registry (sorted by name), reloaded after every change.
    pub agents: Vec<AgentDef>,
    /// Selected index into `agents` (the LIST cursor).
    pub list_sel: usize,
    /// `false` = focus on the LIST pane; `true` = focus on the DETAIL pane.
    pub in_detail: bool,
    /// Active sub-mode (Browse / Edit / Create / DeleteConfirm).
    pub mode: AgentSubMode,
    /// Highlighted field within the Edit/Create editor.
    pub field: AgentEditField,
    /// `true` while typing into the highlighted text field; `false` = navigating.
    pub editing: bool,
    /// Chosen scope for a Create (toggled before naming).
    pub create_scope: AgentScope,
    /// Draft: filename slug (Create only).
    pub draft_name: String,
    /// Draft: description.
    pub draft_description: String,
    /// Draft: conditions (when to delegate to this agent). Optional free text.
    pub draft_conditions: String,
    /// Draft: uuid of the REGISTERED model this agent runs on (`None` = inherit
    /// the Main role). Selected via the single-select model picker; on save it
    /// becomes [`AgentDef::model_uuid`].
    pub draft_model_uuid: Option<String>,
    /// Read-only legacy hint: an old agent file's free-text `model` slug, kept
    /// only so the Model row can SHOW it when the file predates `model_uuid`. It is
    /// never written back (the editor only ever writes `model_uuid`).
    pub draft_model_legacy: Option<String>,
    /// Draft: tool allow-list, raw comma/space separated text.
    pub draft_tools: String,
    /// Draft: markdown body / system prompt.
    pub draft_body: String,
    /// The session's directory (for the Session scope target).
    pub session_dir: PathBuf,
    /// When `Some`, the tool multi-select picker overlay is active.
    ///
    /// All key input is routed to the picker; the form underneath is frozen
    /// until the picker is confirmed (Enter) or cancelled (Esc).
    pub tool_picker: Option<ToolPickerState>,
    /// When `Some`, the single-select MODEL picker overlay is active (opened from
    /// the Model field). Like `tool_picker` it owns all key input while open; the
    /// deepest modal in the agents editor.
    pub model_picker: Option<ModelPickerState>,
    /// When `Some`, the FULL-SCREEN nano-style text editor is active (opened by
    /// activating the Body, Description, or Conditions field). It replaces the
    /// whole agents view and owns all key input; on Esc it commits its text back
    /// into the matching draft (tagged by the [`AgentEditField`]) and closes
    /// (`= None`). A sub-state of `Mode::Agents`, not a separate mode.
    pub editor: Option<(AgentEditField, crate::app::mode::editor::TextEditorState)>,
    /// `true` once Ctrl+X is pressed in the full-screen editor, arming a
    /// "clear the whole field?" confirmation. While armed, `y`/`Y` wipes the
    /// editor buffer and any other key cancels; either way the flag resets. Set
    /// back to `false` whenever the editor opens or commits.
    pub editor_clear_confirm: bool,
}

impl AgentsState {
    /// Build the dashboard from the active session: load the registry and start
    /// in Browse with the LIST focused.
    pub fn from(session: &Session) -> Self {
        let agents = load_agents_snapshot(session);
        Self {
            agents,
            list_sel: 0,
            in_detail: false,
            mode: AgentSubMode::Browse,
            field: AgentEditField::Description,
            editing: false,
            create_scope: AgentScope::Session,
            draft_name: String::new(),
            draft_description: String::new(),
            draft_conditions: String::new(),
            draft_model_uuid: None,
            draft_model_legacy: None,
            draft_tools: String::new(),
            draft_body: String::new(),
            session_dir: session.path.clone(),
            tool_picker: None,
            model_picker: None,
            editor: None,
            editor_clear_confirm: false,
        }
    }

    /// Reload the registry snapshot from disk and clamp the cursor.
    ///
    /// Called after every create/edit/delete so the LIST reflects disk state.
    pub fn reload(&mut self, session: &Session) {
        self.agents = load_agents_snapshot(session);
        if self.list_sel >= self.agents.len() {
            self.list_sel = self.agents.len().saturating_sub(1);
        }
    }

    /// The currently-selected agent, if any.
    pub fn current_agent(&self) -> Option<&AgentDef> {
        self.agents.get(self.list_sel)
    }

    /// The fields visible in the current editor (Create has the name row).
    pub fn fields(&self) -> &'static [AgentEditField] {
        if self.mode == AgentSubMode::Create {
            CREATE_FIELDS
        } else {
            EDIT_FIELDS
        }
    }

    // --- LIST navigation (Browse) ---

    /// Move the LIST cursor up.
    pub fn list_up(&mut self) {
        self.list_sel = self.list_sel.saturating_sub(1);
    }

    /// Move the LIST cursor down.
    pub fn list_down(&mut self) {
        if self.list_sel + 1 < self.agents.len() {
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
    /// The Model field is a picker (never typed into), so it has no text buffer:
    /// the input handler opens the model picker on Enter instead of setting
    /// `editing`, so `draft_mut` is never called while it is highlighted.
    fn draft_mut(&mut self) -> &mut String {
        match self.field {
            AgentEditField::Name => &mut self.draft_name,
            AgentEditField::Description => &mut self.draft_description,
            AgentEditField::Conditions => &mut self.draft_conditions,
            AgentEditField::Tools => &mut self.draft_tools,
            AgentEditField::Body => &mut self.draft_body,
            // Model is a picker, not a text field; it never enters text-edit mode.
            AgentEditField::Model => unreachable!("model field is edited via the picker"),
        }
    }

    /// Immutable handle to the draft buffer for text field `f` (view-side reads).
    ///
    /// The Model field is a picker with no text buffer; the view renders it from
    /// `draft_model_uuid` directly, so it is not requested here.
    pub fn draft(&self, f: AgentEditField) -> &str {
        match f {
            AgentEditField::Name => &self.draft_name,
            AgentEditField::Description => &self.draft_description,
            AgentEditField::Conditions => &self.draft_conditions,
            AgentEditField::Tools => &self.draft_tools,
            AgentEditField::Body => &self.draft_body,
            // Model is a picker, not a text field; the view reads its uuid instead.
            AgentEditField::Model => "",
        }
    }

    /// Append `c` to the active draft. The Name field is restricted to
    /// slug-safe characters (alnum + dash) so the on-screen draft can never hold
    /// a value the data-layer sanitiser would later reject.
    pub fn push_char(&mut self, c: char) {
        if self.field == AgentEditField::Name {
            if c.is_ascii_alphanumeric() || c == '-' {
                self.draft_name.push(c.to_ascii_lowercase());
            }
            return;
        }
        self.draft_mut().push(c);
    }

    /// Delete the last char of the active draft. Body deletes a full char
    /// (including any trailing newline) like every other field.
    pub fn backspace(&mut self) {
        self.draft_mut().pop();
    }

    /// Insert a newline into the body draft (multiline prompt editing).
    pub fn newline(&mut self) {
        if self.field == AgentEditField::Body {
            self.draft_body.push('\n');
        }
    }

    // --- Sub-mode transitions ---

    /// Enter EDIT for the selected agent, seeding drafts from it. Built-in agents
    /// can be edited; saving creates a session-scoped override that preserves
    /// metadata fields not exposed in the editor.
    pub fn enter_edit(&mut self) {
        let Some(a) = self.current_agent().cloned() else {
            return;
        };
        self.draft_name = a.name.clone();
        self.draft_description = a.description.clone();
        self.draft_conditions = a.conditions.clone();
        self.draft_model_uuid = a.model_uuid.clone();
        // Legacy hint only when the file predates `model_uuid` (no registered
        // model chosen, but an old free-text `model` slug is present).
        self.draft_model_legacy = if a.model_uuid.is_none() {
            a.model.clone().filter(|m| !m.trim().is_empty())
        } else {
            None
        };
        self.draft_tools = a.tools.join(", ");
        self.draft_body = a.prompt.clone();
        self.mode = AgentSubMode::Edit;
        self.field = AgentEditField::Description;
        self.in_detail = true;
        self.editing = false;
    }

    /// Enter CREATE: reset every draft, seed the body template, focus the name.
    pub fn enter_create(&mut self) {
        self.draft_name = String::new();
        self.draft_description = String::new();
        self.draft_conditions = String::new();
        self.draft_model_uuid = None;
        self.draft_model_legacy = None;
        self.draft_tools = String::new();
        self.draft_body = TEMPLATE_BODY.to_string();
        self.create_scope = AgentScope::Session;
        self.mode = AgentSubMode::Create;
        self.field = AgentEditField::Name;
        self.in_detail = true;
        self.editing = false;
    }

    /// Enter DELETE-CONFIRM for the selected agent. The caller has already
    /// verified the agent is file-backed (built-ins can never reach here).
    pub fn enter_delete(&mut self) {
        self.mode = AgentSubMode::DeleteConfirm;
        self.editing = false;
    }

    /// Toggle the create scope (Session <-> Global) — Create mode only.
    pub fn toggle_scope(&mut self) {
        self.create_scope = match self.create_scope {
            AgentScope::Session => AgentScope::Global,
            AgentScope::Global => AgentScope::Session,
        };
    }

    /// Discard drafts and return to Browse with the LIST focused.
    pub fn cancel(&mut self) {
        self.mode = AgentSubMode::Browse;
        self.editing = false;
        self.in_detail = false;
        self.field = AgentEditField::Description;
        self.tool_picker = None;
        self.model_picker = None;
        self.editor = None;
    }

    // --- Tool picker overlay ---

    /// Open the tool multi-select picker, seeding checked state from the
    /// current `draft_tools`.
    pub fn open_tool_picker(&mut self) {
        self.tool_picker = Some(ToolPickerState::from_draft(&self.draft_tools));
    }

    /// Confirm the picker: write the selected tools back into `draft_tools`
    /// (comma-joined, options order) and close the overlay.
    pub fn confirm_tool_picker(&mut self) {
        if let Some(p) = self.tool_picker.take() {
            self.draft_tools = p.selected().join(", ");
        }
    }

    /// Cancel the picker without modifying `draft_tools`.
    pub fn cancel_tool_picker(&mut self) {
        self.tool_picker = None;
    }

    // --- Model picker overlay ---

    /// Open the single-select model picker over the registered models, seeding the
    /// cursor from the current `draft_model_uuid`.
    pub fn open_model_picker(&mut self, config: &AppConfig, settings: &Settings) {
        self.model_picker = Some(ModelPickerState::from_models(
            config,
            settings,
            &self.draft_model_uuid,
        ));
    }

    /// Confirm the picker: write the cursor's choice into `draft_model_uuid` and
    /// close the overlay. Choosing a registered model clears the legacy hint (the
    /// agent now resolves through `model_uuid`, so the old slug is no longer shown).
    pub fn confirm_model_picker(&mut self) {
        if let Some(p) = self.model_picker.take() {
            self.draft_model_uuid = p.selected();
            self.draft_model_legacy = None;
        }
    }

    /// Cancel the picker without modifying `draft_model_uuid`.
    pub fn cancel_model_picker(&mut self) {
        self.model_picker = None;
    }

    // --- Full-screen field editor (Body / Description / Conditions) ---

    /// Open the full-screen nano editor for a full-size-editable `field`, seeded
    /// from that field's current draft. Only `Body`, `Description`, and
    /// `Conditions` open this editor; any other field is a no-op.
    ///
    /// Called instead of starting inline editing when the user activates one of
    /// these rows (Enter). While open it owns all key input, tagged by `field`,
    /// and replaces the whole agents view.
    pub fn open_editor(&mut self, field: AgentEditField) {
        let text = match field {
            AgentEditField::Body => &self.draft_body,
            AgentEditField::Description => &self.draft_description,
            AgentEditField::Conditions => &self.draft_conditions,
            _ => return, // only these three are full-size editable
        };
        self.editor = Some((field, crate::app::mode::editor::TextEditorState::from_text(text)));
        // A fresh editor starts with no pending clear-confirm.
        self.editor_clear_confirm = false;
    }

    /// Commit the open editor: write its text back into the draft tagged by the
    /// active field and close the editor (returning to the field list). No-op when
    /// it isn't open.
    pub fn commit_editor(&mut self) {
        if let Some((field, ed)) = self.editor.take() {
            let text = ed.text();
            match field {
                AgentEditField::Body => self.draft_body = text,
                AgentEditField::Description => self.draft_description = text,
                AgentEditField::Conditions => self.draft_conditions = text,
                _ => {}
            }
        }
        // The editor is closing; drop any pending clear-confirm so it can't leak
        // into the next time the editor is opened.
        self.editor_clear_confirm = false;
    }

    /// Build an [`AgentDef`] from the current drafts (the value the runtime
    /// hands to the data layer on create/save).
    ///
    /// `tools` is parsed from the raw draft by splitting on commas/whitespace and
    /// dropping empties — only fields we know how to round-trip are written, so
    /// the on-disk file stays clean. The name is the create draft for Create, or
    /// the selected agent's name for Edit.
    ///
    /// For Edit mode, starts from the current agent to preserve metadata fields
    /// (like `steps`) not exposed in the editor. Built-in agents save as session
    /// overrides; global/session agents preserve their original source. For Create
    /// mode, starts from defaults.
    pub fn to_agent_def(&self) -> AgentDef {
        let name = if self.mode == AgentSubMode::Create {
            self.draft_name.trim().to_string()
        } else {
            self.current_agent()
                .map(|a| a.name.clone())
                .unwrap_or_default()
        };
        let tools: Vec<String> = self
            .draft_tools
            .split([',', ' ', '\t', '\n'])
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        
        // Start from the current agent for Edit mode (to preserve metadata),
        // or from defaults for Create mode.
        let mut def = if self.mode == AgentSubMode::Create {
            AgentDef::default()
        } else {
            self.current_agent().cloned().unwrap_or_default()
        };
        
        // Overwrite the editable fields from drafts.
        def.name = name;
        def.description = self.draft_description.trim().to_string();
        def.conditions = self.draft_conditions.trim().to_string();
        // The chosen registered model (None = inherit Main). The editor no
        // longer writes the legacy `model` / `provider` / `provider_uuid`
        // fields — they stay at their `None` default for new/edited agents.
        def.model_uuid = self.draft_model_uuid.clone();
        def.model = None;
        def.provider = None;
        def.provider_uuid = None;
        def.tools = tools;
        def.prompt = self.draft_body.clone();
        def.source = if self.current_agent().is_some_and(|a| a.source == AgentSource::Builtin) {
            AgentSource::Session
        } else {
            self.current_agent()
                .map(|a| a.source)
                .unwrap_or(AgentSource::Session)
        };
        def.file_path = None;
        
        def
    }
}

/// Source label shown in the LIST and DETAIL panes.
pub fn source_label(source: AgentSource) -> &'static str {
    match source {
        AgentSource::Builtin => "built-in",
        AgentSource::Global => "global",
        AgentSource::Extension => "extension",
        AgentSource::Session => "session",
    }
}

/// Load the registry for a session and flatten it into a sorted owned `Vec`.
fn load_agents_snapshot(session: &Session) -> Vec<AgentDef> {
    let registry = load_registry(Some(&session.path));
    // `list(false)` returns every agent (including hidden) sorted by name; we
    // own a clone so the snapshot survives the registry being dropped.
    registry.list(false).into_iter().cloned().collect()
}
