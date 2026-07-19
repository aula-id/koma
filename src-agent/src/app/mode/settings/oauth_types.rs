//! [`OAuthFlowState`]: the in-progress state machine for the `/settings` OAuth
//! submenu's connect flow (Codex browser login, Codex pasted token, or Kilo
//! Code device login).

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
    /// [`crate::service::oauth::OAuthEvent`] lands.
    Starting,
    /// Provider picker: `0` = Codex (browser), `1` = Kilo Code (browser), `2` =
    /// koma.run (browser), `3` = xAI (browser), `4` = Claude (browser), `5` = Codex (paste token). Inner `usize` is the cursor.
    Pick(usize),
    /// Codex browser flow: the loopback listener is up and `url` is the
    /// authorization URL (shown so the user can copy it if the browser didn't
    /// open). `frame` drives the braille spinner, advanced once per tick.
    /// `copied` flips to `true` after a successful `c` (copy-url) press, so
    /// the view can show a one-shot confirmation line.
    CodexWait { url: String, frame: u8, copied: bool },
    /// Codex manual flow: the user is typing/pasting a raw access token.
    CodexPaste { input: String },
    /// Kilo Code device flow: waiting for the user to approve `user_code` at
    /// `verification_url`. `frame` drives the braille spinner. `copied` flips
    /// to `true` after a successful `c` (copy-url) press.
    KiloWait {
        user_code: String,
        verification_url: String,
        frame: u8,
        copied: bool,
    },
    /// The flow failed; `String` is the human-readable reason. Enter/Esc dismiss
    /// back to `Idle`.
    Failed(String),
}
