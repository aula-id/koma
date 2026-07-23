//! [`OAuthFlowState`]: the in-progress state machine for the `/settings` OAuth
//! submenu's connect flow (Codex browser login, Codex pasted token, or Kilo
//! Code device login).

use crate::model::app_config::OAuthProvider;

/// Where the OAuth "connect" flow currently is. Lives on [`super::SettingsState`]
/// as `oauth_flow`; `Idle` means the OAuth category screen shows the plain
/// connection list (no overlay).
#[derive(Debug, Clone, PartialEq)]
pub enum OAuthFlowState {
    /// No flow in progress — the connection list/`[+connect]` screen.
    Idle,
    /// A flow was just started (`Action::OAuthStart`) but the background task
    /// hasn't reported its first event yet (no URL/code to show). Transitional —
    /// swapped for `CodexWait`/`KiloWait` the moment the corresponding
    /// [`crate::service::oauth::OAuthEvent`] lands. `provider` carries the
    /// chosen provider so the view can render the correct title/port.
    Starting { provider: OAuthProvider },
    /// Provider picker cursor. Indices (must match TUI OPTIONS in
    /// `view/settings/oauth.rs`):
    /// 0 Codex, 1 Kilo Code, 2 koma.run, 3 xAI, 4 Claude,
    /// 5 Command Code, 6 Codex paste, 7 Command Code paste.
    Pick(usize),
    /// Browser-based flow: the loopback listener is up and `url` is the
    /// authorization URL (shown so the user can copy it if the browser didn't
    /// open). Shared by Codex, Claude, Koma, and Command Code browser flows —
    /// `provider` determines the title and port. `frame` drives the braille
    /// spinner, advanced once per tick. `copied` flips to `true` after a
    /// successful `c` (copy-url) press, so the view can show a one-shot
    /// confirmation line.
    CodexWait {
        provider: OAuthProvider,
        url: String,
        frame: u8,
        copied: bool,
    },
    /// Codex manual flow: the user is typing/pasting a raw access token.
    /// `provider` tracks which provider the paste is for (Codex or CommandCode)
    /// so the handler builds the correct conn type.
    CodexPaste {
        input: String,
        provider: OAuthProvider,
    },
    /// Device flow: waiting for the user to approve `user_code` at
    /// `verification_url`. Shared by Kilo Code and xAI device flows —
    /// `provider` determines the title. `frame` drives the braille spinner.
    /// `copied` flips to `true` after a successful `c` (copy-url) press.
    KiloWait {
        provider: OAuthProvider,
        user_code: String,
        verification_url: String,
        frame: u8,
        copied: bool,
    },
    /// The flow failed; `String` is the human-readable reason. Enter/Esc dismiss
    /// back to `Idle`.
    Failed(String),
}
