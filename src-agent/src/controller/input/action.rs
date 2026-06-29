//! The [`Action`] enum: all observable effects a key-press can request from
//! the runtime.  The runtime matches on this value and performs actual state
//! mutations and I/O.

use crate::controller::command::Command;

/// All observable effects a key-press can request from the runtime.
///
/// The runtime match on this value and performs actual state mutations and I/O.
pub enum Action {
    /// Key was recognised but requires no runtime response.
    None,
    /// Exit the application cleanly.
    ///
    /// The runtime's handler is the quit CHOKEPOINT: if any session still has
    /// work in flight it opens the [`crate::app::mode::Mode::QuitConfirm`]
    /// overlay instead of exiting; otherwise it quits immediately (releasing all
    /// locks on the way out via the natural exit path).
    Quit,
    // --- Quit-confirm overlay actions ---
    /// `k` in the quit-confirm overlay: abort EVERY session's in-flight stream
    /// (drop each active receiver + clear each current task), then set
    /// `should_quit`. All on-disk locks are released by the natural exit path.
    QuitKillAll,
    /// `d` in the quit-confirm overlay: detach & quit — set `should_quit`
    /// WITHOUT aborting anything, leaving each session's conversation persisted
    /// on disk (resumable later). Phase 1 caveat: the in-flight work still dies
    /// with the process; true headless detach arrives with the daemon.
    QuitDetach,
    /// `Esc` in the quit-confirm overlay: dismiss it and return to Chat
    /// unchanged. Nothing is aborted and the app keeps running.
    QuitCancel,
    // --- Chat actions ---
    /// User confirmed a non-slash message; inner string is the trimmed input.
    Submit(String),
    /// User ran a `!`-prefixed shell command directly in the session cwd (the `!`
    /// user-shell shortcut). Inner string is the command WITH the leading `!`
    /// stripped + trimmed. The runtime runs it in the foreground session's current
    /// working directory, captures stdout+stderr (same cap/strip/timeout as the
    /// `bash` tool), and appends a distinct shell entry to the conversation — it
    /// does NOT send a turn to the model or start a stream. NOT WC-gated (the user
    /// is trusted; it runs wherever the session cwd currently is).
    Shell(String),
    /// User entered a `/slash` command; inner value is the parsed [`Command`].
    Slash(Command),
    /// Abort an in-flight API request (Ctrl+C / Esc while `waiting = true`).
    Interrupt,
    /// Re-send the last user message (Ctrl+R while idle).
    Resend,
    /// Double-Esc while idle in Chat — open the message-rewind picker. The
    /// runtime builds the [`RewindState`] from the active conversation's user
    /// messages and swaps into `Mode::MessageRewind`. A no-op when there is no
    /// session or no prior user message.
    OpenRewind,
    /// Esc/Ctrl+C in the message-rewind picker — discard it and return to Chat
    /// unchanged (the conversation is untouched).
    RewindCancel,
    /// Enter in the message-rewind picker: rewind the conversation to just
    /// before the highlighted user message (vec index = the inner `usize`) and
    /// load its text into the composer. The runtime truncates the live
    /// `Conversation` (and the sqlite archive) at that boundary, persists, and
    /// drops the message text into the composer WITHOUT auto-sending.
    RewindToMessage(usize),
    /// Approve the paused risky tool call (`y` in the approval modal): run it
    /// and resume the tool-approval state machine.
    ApproveTool,
    /// Deny the paused risky tool call (`n`/Esc in the approval modal): feed
    /// `"denied by user"` back as its result and resume the machine.
    DenyTool,
    // --- KeyInput actions ---
    /// Setup wizard finished; carry the entered endpoint, api key, and model out
    /// so the runtime can build a provider-agnostic config from them.
    SaveCreds { endpoint: String, api_key: String, model: String },
    /// Esc on a credentials form that was NOT opened from the picker — return
    /// to the normal Chat view.
    CancelKeyInput,
    /// Esc from a KeyInput that was opened from the --resume picker: go back to
    /// the picker rather than pinning a no-client Chat.
    CancelKeyInputToPicker,
    // --- Picker actions ---
    /// Esc/Ctrl+C in the session picker opened via /resume (an active session
    /// exists) — discard the picker and return to the unchanged Chat. The
    /// --resume startup picker has no session, so it Quits instead.
    CancelPickerToChat,
    /// Enter on the `--resume` startup session picker — open the highlighted
    /// session (non-destructive: append-or-swap).
    PickerSelect,
    /// `/new` typed in the `--resume` session picker — spawn a fresh session
    /// and jump straight into Chat.
    PickerNewSession,
    // --- Session hub (`/resume`) actions ---
    /// Enter on the hub's COOKING pane: switch the foreground to the live session
    /// at the carried Vec index (`state.rest.sessions[idx]`). The runtime sets
    /// `foreground = idx` and resets the flat foreground-UI for the newly-shown
    /// session WITHOUT aborting anything or touching any lock. Also emitted by the
    /// daemon's UUID-keyed `SwitchForeground` request (resolved to an index).
    LiveSwitch(usize),
    /// Enter on the hub's HISTORY pane: load the on-disk session at the carried
    /// history-row index into a NEW appended tab (non-destructive — the current
    /// foreground keeps cooking). The runtime reads the row's path back out of the
    /// hub state, then runs the same load path as the `--resume` picker (swap if it
    /// turns out to be live, refuse if locked by another process, else load).
    HubOpenHistory(usize),
    /// Confirm a kill armed on the hub's COOKING pane (Enter / y / Ctrl+X while a
    /// `pending_kill` is set). The runtime reads the pending target out of the hub
    /// state and, on a real session, "aborts if cooking, else closes": a working
    /// session is interrupted (kept, goes idle); an idle session is tombstoned
    /// (`close()`), repointing/spawning the foreground if the closed one was it.
    /// The hub is then rebuilt in place (the killed/now-idle session reflected) so
    /// the overlay stays open. No-op if nothing valid is pending.
    HubKillConfirm,
    /// Esc/Ctrl+C on the session hub — close it and return to the (unchanged) Chat
    /// view. No session state is touched.
    CloseSessionHub,
    // --- Settings actions ---
    /// Esc on the settings dashboard (while navigating) — apply every draft and
    /// return to Chat. The apply path reads the drafts back out of
    /// `state.mode`, mirroring [`Action::PickerSelect`].
    SaveSettings,
    // --- Effort picker actions ---
    /// Enter on the `/effort` picker — store the chosen effort, rebuild the
    /// client so it takes effect, and return to Chat. Inner string is the chosen
    /// option (`"default"` stores `""`).
    SaveEffort(String),
    /// Esc on the `/effort` picker — discard the selection and return to Chat.
    EffortCancel,
    // --- Agents dashboard actions ---
    /// Confirm CREATE: write a new agent from the drafts, reload, back to Browse.
    CreateAgent,
    /// Confirm EDIT: overwrite the selected agent from the drafts, reload, back
    /// to Browse.
    SaveAgent,
    /// Confirm DELETE: remove the selected file-backed agent, reload, back to
    /// Browse.
    DeleteAgent,
    /// Esc from the agents dashboard (Browse, LIST focused) — discard any drafts
    /// and return to Chat.
    CloseAgents,
    // --- MCP dashboard actions ---
    /// Confirm CREATE: append a new MCP server from the drafts to the config,
    /// persist, reload, back to Browse.
    CreateMcp,
    /// Confirm EDIT: overwrite the selected MCP server from the drafts, persist,
    /// reload, back to Browse.
    SaveMcp,
    /// Confirm DELETE: remove the selected MCP server from the config, persist,
    /// reload, back to Browse.
    DeleteMcp,
    /// Esc from the MCP dashboard (Browse, LIST focused) — discard any drafts and
    /// return to Chat.
    CloseMcp,
    // --- Security daemon control panel actions ---
    /// Esc from the `/security` panel — return to Chat.
    CloseSecurity,
    /// `t` in the `/security` panel — toggle the security-enabled flag: if now enabled,
    /// start the daemon; if now disabled, stop it. Refreshes the panel status.
    SecurityToggle,
    /// `s` in the `/security` panel — start the daemon (no-op when already running).
    SecurityStart,
    /// `x` in the `/security` panel — stop the daemon (no-op when not running).
    SecurityStop,
    /// `r` in the `/security` panel — restart the daemon (stop then start).
    SecurityRestart,
    /// `Enter`/`Space` in the `/security` panel — toggle the currently-selected tool's
    /// active state (flip its membership in `state.rest.sec_inactive`). A disabled tool
    /// is no longer advertised to the model; re-enabling restores it. Refreshes the panel.
    SecurityToggleTool,
    /// `d` in the `/security` panel — toggle every tool sharing the selected tool's
    /// domain: if all of that domain are currently active, disable them all; otherwise
    /// enable them all. Refreshes the panel.
    SecurityToggleDomain,
    /// `i` in the `/security` panel's DEPENDENCY pane — install/repair the selected
    /// dependency. Inner string is its manifest key (the argument to
    /// [`crate::app::sec::SecDaemonManager::install`]). v1 runs the install BLOCKING
    /// (a Tier-2 download can take seconds), then re-fetches install-health so the
    /// pane's present-flags update.
    SecurityInstall(String),
    // --- Help reference + launcher actions ---
    /// Esc in the `/help` screen (or Enter on a non-launchable keybinding row) —
    /// close the reference and return to Chat unchanged.
    CloseHelp,
    /// Enter on a COMMAND row in `/help`: close the reference AND run the carried
    /// command. The runtime drops back to Chat, then dispatches the [`Command`]
    /// through the SAME `apply_slash` pipeline a typed slash command uses, so the
    /// launcher needs no bespoke plumbing.
    HelpRun(Command),
    /// Fetch the provider-endpoint list for the given model id (the inner
    /// `String`) on a background task. Emitted by the model modal when an
    /// OpenRouter model is selected (search) or an existing model is opened for
    /// edit; the modal's loading flags are already set by the caller. The
    /// runtime opens a fresh `endpoints_rx` channel and spawns the fetch.
    FetchModelEndpoints(String),
    // --- Usage dashboard actions ---
    /// Esc on the usage dashboard — return to Chat.
    CloseUsage,
    // --- Loading splash actions ---
    /// Esc on the startup loading splash — skip the remaining warm steps and drop
    /// straight into Chat. The background warm tasks keep running; their results
    /// still populate `state.rest.*` via the `warm_rx` drain. The handler already
    /// marked any non-terminal step `Skipped` for correctness; the runtime just
    /// swaps the mode to `Chat`.
    SkipLoading,
}
