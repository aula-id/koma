//! Remote host manager state (`/remote`, `Mode::Remote`).

/// Why the remote UI was opened. This controls what selecting a host may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteIntent {
    Manage,
    Resume,
    New,
}

/// Current screen within the remote workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemoteView {
    /// Fullscreen searchable saved-host manager.
    HostManager,
    /// Fullscreen host picker used by remote resume/new.
    HostPicker,
    /// Management-only host details and diagnostics.
    HostDetail,
    /// Existing sessions on the prepared remote host.
    SessionHub,
    /// Create a new host (form fields).
    CreateHost,
    /// Edit an existing host (form fields).
    EditHost,
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
    /// Search/filter query.
    pub query: String,
    /// Indices matching the current query.
    pub filtered: Vec<usize>,
    /// Host ID shown in management detail.
    pub detail_host: Option<String>,
    /// Host ID selected for preparation or session discovery.
    pub selected_host_id: Option<String>,
    /// Transient connection state (while preparing / authenticating / connecting).
    pub connection_state: Option<ConnectionState>,
    /// Sessions discovered on the selected host.
    pub sessions: Vec<RemoteSession>,
    /// Selected session index.
    pub session_selected: usize,
    /// Host ID pending delete confirmation.
    pub pending_delete: Option<String>,
    /// Password input buffer (masked in rendering).
    pub password_buf: String,
    /// Host editor state (Some when in CreateHost/EditHost sub).
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
            view: if intent == RemoteIntent::Manage {
                RemoteView::HostManager
            } else {
                RemoteView::HostPicker
            },
            selected: 0,
            hosts,
            filtered,
            query: String::new(),
            detail_host: None,
            selected_host_id: None,
            connection_state: None,
            sessions: Vec::new(),
            session_selected: 0,
            pending_delete: None,
            password_buf: String::new(),
            editor: None,
            editing_field: false,
        }
    }

    /// Refilter the host list based on the current query.
    pub fn refilter(&mut self) {
        if self.query.is_empty() {
            self.filtered = (0..self.hosts.len()).collect();
        } else {
            let q = self.query.to_lowercase();
            self.filtered = self
                .hosts
                .iter()
                .enumerate()
                .filter(|(_, h)| {
                    h.name.to_lowercase().contains(&q)
                        || h.host.to_lowercase().contains(&q)
                        || h.user.to_lowercase().contains(&q)
                        || h.tags.iter().any(|t| t.to_lowercase().contains(&q))
                })
                .map(|(i, _)| i)
                .collect();
        }
        // Clamp selection.
        if self.selected >= self.filtered.len() {
            self.selected = self.filtered.len().saturating_sub(1);
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

    /// Open management detail for the currently selected host.
    pub fn enter_detail(&mut self) {
        if self.intent != RemoteIntent::Manage {
            return;
        }
        if let Some(host) = self.selected_host() {
            self.detail_host = Some(host.id.clone());
            self.view = RemoteView::HostDetail;
        }
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
        self.view = RemoteView::CreateHost;
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
            self.view = RemoteView::EditHost;
        }
    }

    /// Cancel editing and return to the management root.
    pub fn cancel_edit(&mut self) {
        self.editor = None;
        self.editing_field = false;
        self.view = RemoteView::HostManager;
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
mod tests {
    use super::*;

    fn make_test_host(id: &str, name: &str) -> crate::remote::hosts::RemoteHost {
        crate::remote::hosts::RemoteHost {
            id: id.into(),
            name: name.into(),
            user: "root".into(),
            host: "10.0.0.1".into(),
            port: 22,
            key_path: None,
            last_connected: None,
            tags: vec![],
        }
    }

    #[test]
    fn manage_intent_starts_at_host_manager() {
        let hosts = vec![make_test_host("h1", "srv")];
        let state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
        assert_eq!(state.intent, RemoteIntent::Manage);
        assert_eq!(state.view, RemoteView::HostManager);
    }

    #[test]
    fn resume_intent_starts_at_host_picker() {
        let state = RemoteState::for_intent(vec![], RemoteIntent::Resume);
        assert_eq!(state.intent, RemoteIntent::Resume);
        assert_eq!(state.view, RemoteView::HostPicker);
    }

    #[test]
    fn new_intent_starts_at_host_picker() {
        let state = RemoteState::for_intent(vec![], RemoteIntent::New);
        assert_eq!(state.intent, RemoteIntent::New);
        assert_eq!(state.view, RemoteView::HostPicker);
    }

    #[test]
    fn new_alias_starts_at_host_manager() {
        let state = RemoteState::new(vec![]);
        assert_eq!(state.intent, RemoteIntent::Manage);
        assert_eq!(state.view, RemoteView::HostManager);
    }

    #[test]
    fn enter_detail_only_for_manage_intent() {
        let hosts = vec![make_test_host("h1", "srv")];

        // Manage intent: enter_detail transitions to HostDetail.
        let mut state = RemoteState::for_intent(hosts.clone(), RemoteIntent::Manage);
        state.enter_detail();
        assert_eq!(state.view, RemoteView::HostDetail);
        assert_eq!(state.detail_host.as_deref(), Some("h1"));

        // Resume intent: enter_detail is a no-op.
        let mut state = RemoteState::for_intent(hosts, RemoteIntent::Resume);
        state.enter_detail();
        assert_eq!(state.view, RemoteView::HostPicker);
        assert!(state.detail_host.is_none());
    }

    #[test]
    fn enter_create_transitions_to_create_host() {
        let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
        state.enter_create();
        assert_eq!(state.view, RemoteView::CreateHost);
        assert!(state.editor.is_some());
        assert!(!state.editing_field);
    }

    #[test]
    fn enter_edit_transitions_to_edit_host() {
        let hosts = vec![make_test_host("h1", "srv")];
        let mut state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
        state.enter_detail();
        state.enter_edit();
        assert_eq!(state.view, RemoteView::EditHost);
        assert!(state.editor.is_some());
        let editor = state.editor.as_ref().unwrap();
        assert_eq!(editor.edit_id.as_deref(), Some("h1"));
        assert_eq!(editor.name, "srv");
    }

    #[test]
    fn cancel_edit_returns_to_host_manager() {
        let hosts = vec![make_test_host("h1", "srv")];
        let mut state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
        state.enter_detail();
        state.enter_edit();
        assert_eq!(state.view, RemoteView::EditHost);
        state.cancel_edit();
        assert_eq!(state.view, RemoteView::HostManager);
        assert!(state.editor.is_none());
    }

    #[test]
    fn validate_editor_rejects_empty_fields() {
        let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
        state.enter_create();
        assert!(!state.validate_editor());
        assert!(state.editor.as_ref().unwrap().error.is_some());
    }

    #[test]
    fn validate_editor_rejects_invalid_port() {
        let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
        state.enter_create();
        if let Some(ref mut editor) = state.editor {
            editor.name = "test".into();
            editor.user = "root".into();
            editor.host = "10.0.0.1".into();
            editor.port = "not_a_number".into();
        }
        assert!(!state.validate_editor());
        assert_eq!(
            state.editor.as_ref().unwrap().error.as_deref(),
            Some("port must be a number")
        );
    }

    #[test]
    fn validate_editor_accepts_valid_fields() {
        let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
        state.enter_create();
        if let Some(ref mut editor) = state.editor {
            editor.name = "test".into();
            editor.user = "root".into();
            editor.host = "10.0.0.1".into();
            editor.port = "22".into();
        }
        assert!(state.validate_editor());
        assert!(state.editor.as_ref().unwrap().error.is_none());
    }

    #[test]
    fn build_host_from_editor() {
        let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
        state.enter_create();
        if let Some(ref mut editor) = state.editor {
            editor.name = "prod".into();
            editor.user = "deploy".into();
            editor.host = "example.com".into();
            editor.port = "2222".into();
            editor.key_path = "/tmp/key".into();
        }
        let host = state.build_host().expect("build_host should succeed");
        assert_eq!(host.name, "prod");
        assert_eq!(host.user, "deploy");
        assert_eq!(host.host, "example.com");
        assert_eq!(host.port, 2222);
        assert_eq!(host.key_path.as_deref(), Some("/tmp/key"));
    }

    #[test]
    fn build_host_empty_key_path_is_none() {
        let mut state = RemoteState::for_intent(vec![], RemoteIntent::Manage);
        state.enter_create();
        if let Some(ref mut editor) = state.editor {
            editor.name = "test".into();
            editor.user = "root".into();
            editor.host = "10.0.0.1".into();
            editor.port = "22".into();
            editor.key_path = "  ".into(); // whitespace only
        }
        let host = state.build_host().expect("build_host should succeed");
        assert!(host.key_path.is_none());
    }

    #[test]
    fn refilter_matches_name_host_user_tags() {
        let hosts = vec![
            make_test_host("h1", "prod-server"),
            make_test_host("h2", "staging-server"),
            make_test_host("h3", "dev"),
        ];
        let mut state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
        // Search by name substring.
        state.query = "prod".into();
        state.refilter();
        assert_eq!(state.filtered, vec![0]);

        // Search by user (all have "root").
        state.query = "root".into();
        state.refilter();
        assert_eq!(state.filtered.len(), 3);

        // Empty query: all match.
        state.query.clear();
        state.refilter();
        assert_eq!(state.filtered.len(), 3);
    }

    #[test]
    fn move_up_down_clamps() {
        let hosts = vec![make_test_host("h1", "a"), make_test_host("h2", "b")];
        let mut state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
        // Starts at 0.
        assert_eq!(state.selected, 0);
        state.move_up(); // Already at top, no-op.
        assert_eq!(state.selected, 0);
        state.move_down();
        assert_eq!(state.selected, 1);
        state.move_down(); // Already at bottom, no-op.
        assert_eq!(state.selected, 1);
        state.move_up();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn selected_host_returns_correct_host() {
        let hosts = vec![
            make_test_host("h1", "first"),
            make_test_host("h2", "second"),
        ];
        let state = RemoteState::for_intent(hosts, RemoteIntent::Manage);
        assert_eq!(
            state.selected_host().map(|h| h.name.as_str()),
            Some("first")
        );
    }

    #[test]
    fn host_edit_field_cycle() {
        assert_eq!(HostEditField::Name.next(), HostEditField::User);
        assert_eq!(HostEditField::User.next(), HostEditField::Host);
        assert_eq!(HostEditField::Host.next(), HostEditField::Port);
        assert_eq!(HostEditField::Port.next(), HostEditField::KeyPath);
        assert_eq!(HostEditField::KeyPath.next(), HostEditField::Name);

        assert_eq!(HostEditField::Name.prev(), HostEditField::KeyPath);
        assert_eq!(HostEditField::User.prev(), HostEditField::Name);
        assert_eq!(HostEditField::Host.prev(), HostEditField::User);
        assert_eq!(HostEditField::Port.prev(), HostEditField::Host);
        assert_eq!(HostEditField::KeyPath.prev(), HostEditField::Port);
    }

    #[test]
    fn connection_state_transitions_covered() {
        // Verify all ConnectionState variants are present and distinct.
        let states = [
            ConnectionState::Disconnected,
            ConnectionState::Resolving,
            ConnectionState::Authenticating,
            ConnectionState::AuthRequired {
                host_id: "x".into(),
                user: "u".into(),
                host: "h".into(),
            },
            ConnectionState::Bootstrapping,
            ConnectionState::Connecting,
            ConnectionState::Connected {
                session_id: "s".into(),
            },
            ConnectionState::Error {
                message: "e".into(),
            },
        ];
        // All distinct (clone + eq check).
        for (i, a) in states.iter().enumerate() {
            for (j, b) in states.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b);
                }
            }
        }
    }
}
