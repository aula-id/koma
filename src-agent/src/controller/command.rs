//! Controller – `/slash` command parser.
//!
//! When the user types a line starting with `/` in Chat mode,
//! [`controller::input`] calls [`parse`] to turn the raw text into a
//! [`Command`] value.  The runtime then routes that value to the appropriate
//! service logic (compaction, new session, rename, etc.).
//!
//! Supported commands: `/compact`, `/new`, `/mode`, `/effort`,
//! `/rename [session] <name>`, `/settings` (alias `/config`),
//! `/resume` (alias `/sessions`), `/task <agent> <task>`,
//! `/cd <path>`, `/adddir <path>`,
//! `/internet [simple|full]`, `/help`, `/quit` (aliases: `/q`, `/exit`).

use crate::model::settings::InternetMode;

/// User-facing slash commands shown in the `/` palette, in display order.
/// (name, one-line description). Source of truth for the palette UI.
pub const COMMANDS: &[(&str, &str)] = &[
    ("/new", "Spawn a new parallel session (current keeps running)"),
    ("/resume", "Open the session hub (live + past sessions)"),
    ("/mode", "Toggle Normal/Auto tool approval"),
    ("/effort", "Set model reasoning/thinking effort"),
    ("/internet", "Toggle internet mode (simple | full)"),
    ("/settings", "Edit key, model, provider, theme, name"),
    ("/agents", "Create, modify, or delete agent definitions"),
    ("/task", "Run an agent on a task in the background"),
    ("/cd", "Change the session working directory"),
    ("/adddir", "Add a directory to the workspace roots"),
    ("/compact", "Summarize and compact the conversation"),
    ("/usage", "Show the cost and token usage dashboard"),
    ("/rename", "Rename the current session"),
    ("/select", "Dump history to the terminal to copy/paste"),
    ("/help", "List the available commands"),
    ("/quit", "Quit koma"),
];

/// True while the user is still typing a command NAME: input starts with `/`
/// and contains no whitespace yet (once they type a space they're onto args).
pub fn palette_active(input: &str) -> bool {
    input.starts_with('/') && !input.contains(char::is_whitespace)
}

/// Commands whose name starts with the typed prefix (case-insensitive).
/// Empty when the palette isn't active.
pub fn palette_matches(input: &str) -> Vec<(&'static str, &'static str)> {
    if !palette_active(input) {
        return Vec::new();
    }
    let prefix = input.to_lowercase();
    COMMANDS
        .iter()
        .filter(|(name, _)| name.starts_with(&prefix))
        .copied()
        .collect()
}

/// A parsed in-chat slash command.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Compact the conversation history to save context window space.
    Compact,
    /// Spawn a fresh PARALLEL session (the current one keeps running in the
    /// background); the new session becomes the foreground.
    New,
    /// Toggle the tool-approval policy between Normal and Auto.
    Mode,
    /// Open the reasoning/thinking-effort picker for the current model.
    Effort,
    /// Rename the current session.  Holds the new name string.
    Rename(String),
    /// Open the in-app settings dashboard (alias: `/config`).
    Settings,
    /// Open the `/agents` management dashboard (alias: `/agent`).
    Agents,
    /// Run a named agent on a task in the background. Holds `<agent> <task>`.
    Task(String),
    /// Change the session's working directory to the held path (Phase 8). The
    /// USER path is UNRESTRICTED — no workspace allow-list check (the user is
    /// trusted); resolution is shell-like (`[N]` / absolute / relative-to-cwd).
    Cd(String),
    /// Append the held directory to the session's workspace roots (widen the
    /// allow-list / add an `[N]` root). Resolved relative to the current cwd.
    AddDir(String),
    /// Toggle or set internet mode. `None` = toggle; `Some(mode)` = set explicitly.
    Internet(Option<InternetMode>),
    /// Open the unified session hub — live (cooking) + past (history) sessions in
    /// one two-pane overlay (alias: `/sessions`).
    Resume,
    /// Dump the conversation to the normal terminal for native copy/paste.
    Select,
    /// Print available commands to the chat view.
    Help,
    /// Open the usage dashboard (`/usage`).
    Usage,
    /// Exit the application.
    Quit,
    /// An unrecognised command verb; holds the raw verb for display.
    Unknown(String),
}

/// Parse a slash-command from `line`.
///
/// `line` is the raw user input — already known to start with `/`.
/// The verb is matched case-insensitively; the remainder preserves original
/// casing so that session names are not lowercased.
///
/// `/rename session <name>` and `/rename <name>` are both accepted: the
/// optional literal word `"session"` is stripped from the remainder before
/// the name is extracted.
pub fn parse(line: &str) -> Command {
    let trimmed = line.trim();
    let without = trimmed.strip_prefix('/').unwrap_or(trimmed);

    // Split off the verb (first whitespace-delimited token).
    let head = without.split_whitespace().next().unwrap_or("").to_string();
    let head_lc = head.to_lowercase();

    // `rest` is sliced from the original-cased `without` so that everything
    // after the verb keeps its capitalisation (important for session names).
    let rest = without[head.len()..].trim_start();

    match head_lc.as_str() {
        "compact" => Command::Compact,
        "new" => Command::New,
        "mode" => Command::Mode,
        "effort" => Command::Effort,
        "settings" | "config" => Command::Settings,
        "agents" | "agent" => Command::Agents,
        "task" => Command::Task(rest.to_string()),
        "cd" => Command::Cd(rest.to_string()),
        "adddir" => Command::AddDir(rest.to_string()),
        "internet" => Command::Internet(InternetMode::from_token(rest)),
        "resume" | "sessions" => Command::Resume,
        "select" => Command::Select,
        "help" => Command::Help,
        "usage" => Command::Usage,
        "quit" | "q" | "exit" => Command::Quit,
        "rename" => {
            // Accept "/rename session <name>" as well as "/rename <name>".
            // Strip the optional literal "session" prefix from the remainder.
            let name = rest.strip_prefix("session").map(str::trim).unwrap_or(rest);
            Command::Rename(name.trim().to_string())
        }
        other => Command::Unknown(other.to_string()),
    }
}
