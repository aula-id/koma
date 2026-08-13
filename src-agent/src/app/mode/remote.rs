//! Remote host manager state (`/remote`, `Mode::Remote`).

/// Which sub-view within the remote manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum RemoteSub {
    /// Compact overlay above composer (host list).
    Compact,
    /// Fullscreen detail view (host detail + sessions).
    Fullscreen,
    /// Connection progress view.
    Connecting,
    /// Inline masked password input.
    PasswordInput,
}

/// Connection stage during SSH connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ConnectStage {
    Resolving,
    Authenticating,
    Bootstrapping,
    Connected,
}

impl ConnectStage {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Resolving => "resolving",
            Self::Authenticating => "authenticating",
            Self::Bootstrapping => "bootstrapping",
            Self::Connected => "connected",
        }
    }
}

/// Connection status for a remote host.
#[derive(Debug, Clone)]
pub struct ConnectionStatus {
    pub stage: ConnectStage,
    pub error: Option<String>,
}

/// State for the remote host manager mode.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RemoteState {
    /// Current sub-view.
    pub sub: RemoteSub,
    /// All saved hosts.
    pub hosts: Vec<crate::remote::hosts::RemoteHost>,
    /// Selected index in the host list.
    pub selected: usize,
    /// Search/filter query.
    pub query: String,
    /// Indices matching the current query.
    pub filtered: Vec<usize>,
    /// Host ID when in Fullscreen sub.
    pub detail_host: Option<String>,
    /// Connection status (while Connecting).
    pub connection_status: Option<ConnectionStatus>,
    /// Sessions on the selected host (placeholder for now).
    pub sessions: Vec<RemoteSession>,
    /// Selected session index.
    pub session_selected: usize,
    /// Host ID pending delete confirmation.
    pub pending_delete: Option<String>,
    /// Password input buffer (masked in rendering).
    pub password_buf: String,
    /// Host being connected to (for password prompt).
    pub connecting_host: Option<String>,
}

/// A session on a remote host (placeholder — will be populated by SSH query).
#[derive(Debug, Clone)]
pub struct RemoteSession {
    pub session_id: String,
    pub name: String,
    pub working: bool,
    pub is_foreground: bool,
}

impl RemoteState {
    /// Build a new RemoteState from the persisted hosts.
    pub fn new(hosts: Vec<crate::remote::hosts::RemoteHost>) -> Self {
        let filtered: Vec<usize> = (0..hosts.len()).collect();
        Self {
            sub: RemoteSub::Compact,
            selected: 0,
            hosts,
            filtered,
            query: String::new(),
            detail_host: None,
            connection_status: None,
            sessions: Vec::new(),
            session_selected: 0,
            pending_delete: None,
            password_buf: String::new(),
            connecting_host: None,
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

    /// Get the currently selected host mutably.
    pub fn selected_host_mut(&mut self) -> Option<&mut crate::remote::hosts::RemoteHost> {
        let idx = *self.filtered.get(self.selected)?;
        Some(&mut self.hosts[idx])
    }

    /// Enter fullscreen for the currently selected host.
    pub fn enter_fullscreen(&mut self) {
        if let Some(host) = self.selected_host() {
            self.detail_host = Some(host.id.clone());
            self.sub = RemoteSub::Fullscreen;
        }
    }
}
