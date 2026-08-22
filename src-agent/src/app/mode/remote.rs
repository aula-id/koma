//! Remote host manager state (`/remote`, `Mode::Remote`).

/// Why the remote UI was opened. This controls what selecting a host may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteIntent {
    Manage,
    Resume,
    New,
}

/// Current screen within the remote workflow.
///
/// Mirrors the `/agents` two-pane pattern: `Browse` shows a list sidebar + detail
/// pane; `Edit` takes over the full screen for create/edit form; `SessionHub` is
/// an overlay session picker; `DeleteConfirm` is a modal on top of Browse.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteView {
    /// Browse hosts: list sidebar + detail pane.
    Browse,
    /// Create or edit a host (form fields take over the full screen).
    Edit,
    /// Resume session picker (overlay on top of Browse).
    SessionHub,
    /// Delete confirmation modal (overlay on top of Browse).
    DeleteConfirm,
}

/// Which field is focused in the create/edit form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEditField {
    Name,
    User,
    Host,
    Port,
    KeyPath,
}

impl HostEditField {
    pub fn next(self) -> Self {
        match self {
            Self::Name => Self::User,
            Self::User => Self::Host,
            Self::Host => Self::Port,
            Self::Port => Self::KeyPath,
            Self::KeyPath => Self::Name,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Name => Self::KeyPath,
            Self::User => Self::Name,
            Self::Host => Self::User,
            Self::Port => Self::Host,
            Self::KeyPath => Self::Port,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "name",
            Self::User => "user",
            Self::Host => "host",
            Self::Port => "port",
            Self::KeyPath => "key path",
        }
    }
}

/// Editor state for creating/editing a host.
#[derive(Debug, Clone)]
pub struct HostEditor {
    /// The draft field values.
    pub name: String,
    pub user: String,
    pub host: String,
    pub port: String, // String for editing, parsed to u16 on save
    pub key_path: String,
    /// Which field is focused.
    pub focused: HostEditField,
    /// Whether we're editing an existing host (Some(id)) or creating new (None).
    pub edit_id: Option<String>,
    /// Validation error, if any.
    pub error: Option<String>,
}

/// Transient connection state for a remote host (lives only in memory, never persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Resolving,
    Authenticating,
    AuthRequired {
        host_id: String,
        user: String,
        host: String,
    },
    Bootstrapping,
    Connecting,
    Connected {
        session_id: String,
    },
    Error {
        message: String,
    },
}

/// State for the remote host manager mode.
#[derive(Debug, Clone)]
pub struct RemoteState {
    /// Purpose of this remote workflow.
    pub intent: RemoteIntent,
    /// Current screen.
    pub view: RemoteView,
    /// All saved hosts.
    pub hosts: Vec<crate::remote::hosts::RemoteHost>,
    /// Selected index in the host list.
    pub selected: usize,
    /// Indices matching the current query (always all indices — kept for wire compat).
    pub filtered: Vec<usize>,
    /// Host ID selected for preparation or session discovery.
    pub selected_host_id: Option<String>,
    /// Transient connection state (while preparing / authenticating / connecting).
    pub connection_state: Option<ConnectionState>,
    /// Sessions discovered on the selected host.
    pub sessions: Vec<RemoteSession>,
    /// Selected session index.
    pub session_selected: usize,
    /// Password input buffer (masked in rendering).
    pub password_buf: String,
    /// Host editor state (Some when in Edit sub-mode).
    pub editor: Option<HostEditor>,
    /// Whether the editor is in text-edit mode (actively typing into a field).
    pub editing_field: bool,
}

/// A discovered session on a remote host.
#[derive(Debug, Clone)]
pub struct RemoteSession {
    pub session_id: String,
    pub name: String,
    pub working: bool,
    pub is_foreground: bool,
}

impl RemoteState {
    /// Build the fullscreen management workflow from persisted hosts.
    pub fn new(hosts: Vec<crate::remote::hosts::RemoteHost>) -> Self {
        Self::for_intent(hosts, RemoteIntent::Manage)
    }

    /// Build a remote workflow for management, resume, or new-session intent.
    pub fn for_intent(hosts: Vec<crate::remote::hosts::RemoteHost>, intent: RemoteIntent) -> Self {
        let filtered: Vec<usize> = (0..hosts.len()).collect();
        Self {
            intent,
            view: RemoteView::Browse,
            selected: 0,
            hosts,
            filtered,
            selected_host_id: None,
            connection_state: None,
            sessions: Vec::new(),
            session_selected: 0,
            password_buf: String::new(),
            editor: None,
            editing_field: false,
        }
    }

    /// Move selection up.
    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    /// Move selection down.
    pub fn move_down(&mut self) {
        if self.selected + 1 < self.filtered.len() {
            self.selected += 1;
        }
    }

    /// Get the currently selected host (by filtered index).
    pub fn selected_host(&self) -> Option<&crate::remote::hosts::RemoteHost> {
        self.filtered
            .get(self.selected)
            .and_then(|&idx| self.hosts.get(idx))
    }

    /// Select the current host for a resume/new workflow.
    pub fn select_current_host(&mut self) -> Option<String> {
        let host_id = self.selected_host()?.id.clone();
        self.selected_host_id = Some(host_id.clone());
        Some(host_id)
    }

    /// Enter the create-host form.
    pub fn enter_create(&mut self) {
        self.editor = Some(HostEditor {
            name: String::new(),
            user: "root".into(),
            host: String::new(),
            port: "22".into(),
            key_path: String::new(),
            focused: HostEditField::Name,
            edit_id: None,
            error: None,
        });
        self.editing_field = false;
        self.view = RemoteView::Edit;
    }

    /// Enter the edit-host form for the currently selected host.
    pub fn enter_edit(&mut self) {
        if let Some(host) = self.selected_host().cloned() {
            self.editor = Some(HostEditor {
                name: host.name.clone(),
                user: host.user.clone(),
                host: host.host.clone(),
                port: host.port.to_string(),
                key_path: host.key_path.clone().unwrap_or_default(),
                focused: HostEditField::Name,
                edit_id: Some(host.id.clone()),
                error: None,
            });
            self.editing_field = false;
            self.view = RemoteView::Edit;
        }
    }

    /// Enter the delete confirmation modal for the currently selected host.
    pub fn enter_delete(&mut self) {
        if self.selected_host().is_some() {
            self.view = RemoteView::DeleteConfirm;
        }
    }

    /// Cancel editing and return to the browse root.
    pub fn cancel_edit(&mut self) {
        self.editor = None;
        self.editing_field = false;
        self.view = RemoteView::Browse;
    }

    /// Validate the editor fields. Returns true if all fields are valid.
    pub fn validate_editor(&mut self) -> bool {
        let Some(editor) = &mut self.editor else {
            return false;
        };
        editor.error = None;

        if editor.name.trim().is_empty() {
            editor.error = Some("name is required".into());
            editor.focused = HostEditField::Name;
            return false;
        }
        if editor.user.trim().is_empty() {
            editor.error = Some("user is required".into());
            editor.focused = HostEditField::User;
            return false;
        }
        if editor.host.trim().is_empty() {
            editor.error = Some("host is required".into());
            editor.focused = HostEditField::Host;
            return false;
        }
        match editor.port.parse::<u16>() {
            Ok(0) => {
                editor.error = Some("port must be non-zero".into());
                editor.focused = HostEditField::Port;
                false
            }
            Ok(_) => true,
            Err(_) => {
                editor.error = Some("port must be a number".into());
                editor.focused = HostEditField::Port;
                false
            }
        }
    }

    /// Build a RemoteHost from the current editor state.
    pub fn build_host(&self) -> Option<crate::remote::hosts::RemoteHost> {
        let editor = self.editor.as_ref()?;
        let port = editor.port.parse::<u16>().ok()?;
        let key_path = if editor.key_path.trim().is_empty() {
            None
        } else {
            Some(editor.key_path.trim().to_string())
        };
        let id = editor
            .edit_id
            .clone()
            .unwrap_or_else(crate::model::app_config::new_uuid);

        // Preserve tags and last_connected from existing host on edit
        let (tags, last_connected) = if let Some(ref edit_id) = editor.edit_id {
            if let Some(existing) = self.hosts.iter().find(|h| h.id == *edit_id) {
                (existing.tags.clone(), existing.last_connected)
            } else {
                (vec![], None)
            }
        } else {
            (vec![], None)
        };

        Some(crate::remote::hosts::RemoteHost {
            id,
            name: editor.name.trim().to_string(),
            user: editor.user.trim().to_string(),
            host: editor.host.trim().to_string(),
            port,
            key_path,
            last_connected,
            tags,
        })
    }
}

#[cfg(test)]
#[path = "remote_test.rs"]
mod tests;
