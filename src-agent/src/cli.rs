//! CLI argument parsing.
//!
//! # User-facing surface (what `--help` should advertise)
//!
//! - `koma`                        — default: spawn-or-attach the daemon, then run the client.
//! - `koma agents`                 — open the session hub (friendly alias for `--resume`).
//! - `koma --resume`               — open the session hub.
//! - `koma alone`                  — standalone no-daemon TUI (friendly alias for `--local`).
//! - `koma daemon <status|kill|restart|clean>` — daemon management CLI.
//! - `koma ext install --dev <zip|dir>` — sideload an unsigned local extension
//!   (dev-only; `koma ext`/`koma ext install` with no `--dev` prints usage — the
//!   in-app store is the normal, signed install path).
//! - `koma doctor [-v|--verbose]` — flutter-doctor-style readiness report
//!   (config/daemons/models/GUI deps/extensions/optional subsystems/MCP/update);
//!   strictly read-only, exits non-zero only on a hard failure.
//! - `--internet-fullmode-install` — provision the Python full-mode (browser) environment and exit.
//! - `--internet-fullmode-uninstall` — remove the Python full-mode environment and exit.
//! - `--force`                     — modifier for `--internet-fullmode-install`: force a reinstall
//!   even when the environment is already present.
//!
//! # Hidden plumbing (still parsed + functional, just not advertised)
//!
//! These remain so the default launch / management paths can drive them internally
//! (e.g. `spawn_daemon` execs `koma --daemon`), but they are no longer surfaced to users
//! — the friendly verbs above front them:
//! - `--resume` is fronted by `agents`; `--local` is fronted by `alone`.
//! - `--daemon`         — run the headless koma-daemon event loop (no terminal).
//! - `--attach`         — run as a thin client that attaches to a running daemon.
//! - `--local`          — force the OLD standalone local TUI (the escape hatch from the
//!   daemon-by-default launch). Refuses to run if a daemon is already alive (it would corrupt the
//!   daemon's session locks); use plain `koma` to attach, or `koma daemon kill` first.
//! - `--ipc-selftest`   — round-trip the daemon IPC transport end-to-end, then exit.
//! - `--daemon-selftest` — drive the full daemon stack (bind/accept/per-client/loop) end-to-end, then exit.
//!
//! # Positional verbs / subcommands (distinct from the `--flags`)
//!
//! - `agents` — alias for `--resume` (open the session hub). Sets the same `resume` bit.
//! - `alone`  — alias for `--local` (standalone TUI). Sets the same `local` bit, so `main`
//!   reuses the identical `--local` branch, daemon-alive guard included.
//! - `daemon <status|kill|restart|clean>` — the daemon management CLI (#118). Parsed
//!   into [`Opts::subcommand`] and short-circuited in `main` BEFORE the TUI, so it
//!   works even when the TUI can't start.
//!
//! `parse` accepts anything that yields `String` items so it can be called
//! with `std::env::args()` directly from `main`.

/// A `koma daemon <verb>` management subcommand (#118).
///
/// Parsed POSITIONALLY (the literal token `daemon` followed by a verb), separate
/// from the `--flags`. `main` short-circuits these BEFORE starting the TUI, so they
/// must run even when the terminal can't be set up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonSub {
    /// `koma daemon status` — report whether a daemon is live (+ PID / socket / session count).
    Status,
    /// `koma daemon kill` — gracefully stop a running daemon (escalating to signals if it won't die).
    Kill,
    /// `koma daemon restart` — kill then spawn a fresh detached daemon, reporting the new PID.
    Restart,
    /// `koma daemon clean` — nuke a stale socket/pidfile when NO daemon is running (refuses if one is).
    Clean,
}

impl DaemonSub {
    /// Map a verb token (`status`/`kill`/`restart`/`clean`) to a [`DaemonSub`].
    /// Returns `None` for anything else so the caller can print usage rather than guess.
    fn from_verb(verb: &str) -> Option<Self> {
        match verb {
            "status" => Some(DaemonSub::Status),
            "kill" => Some(DaemonSub::Kill),
            "restart" => Some(DaemonSub::Restart),
            "clean" => Some(DaemonSub::Clean),
            _ => None,
        }
    }
}

/// The outcome of detecting a `koma daemon …` invocation.
///
/// One field on [`Opts`] captures all three cases so `main` can short-circuit the
/// TUI for ANY `daemon` invocation (valid OR malformed) — a bare/unknown verb must
/// print usage and exit, NOT silently fall through to launching the terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonCli {
    /// `daemon <valid-verb>` — run that management action.
    Run(DaemonSub),
    /// `daemon` with no verb or an unrecognised one — print usage and exit non-zero.
    Usage,
}

/// The outcome of detecting a `koma ext …` invocation (dev-sideload verb family).
///
/// Mirrors [`DaemonCli`]'s shape: `main` short-circuits the TUI for ANY `ext`
/// invocation, valid or malformed. Today the only real verb is `install --dev
/// <path>`; `ext`, `ext install` (no `--dev`), or anything else prints usage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtCli {
    /// `ext install --dev <path>` — sideload an unsigned `.zip` or directory from
    /// local disk into `~/.koma/extensions/<id>/`, no store/signature/signin.
    InstallDev(String),
    /// `ext` alone, `ext install` without `--dev`, or an unrecognised verb — print
    /// usage and exit non-zero.
    Usage,
}

/// Parsed command-line options passed through to the runtime.
#[derive(Debug, Clone, Default)]
pub struct Opts {
    /// When `true`, show the session picker on startup (`--resume` flag).
    pub resume: bool,
    /// When `true`, provision the Python full-mode (browser) environment then exit.
    pub internet_fullmode_install: bool,
    /// When `true`, remove the Python full-mode (browser) environment then exit.
    pub internet_fullmode_uninstall: bool,
    /// When `true`, provision the Python security daemon environment then exit.
    pub security_install: bool,
    /// Modifier for `--internet-fullmode-install` / `--security-install`: overwrite an existing install.
    pub force: bool,
    /// When `true`, run the daemon IPC transport self-test then exit
    /// (`--ipc-selftest` flag).
    pub ipc_selftest: bool,
    /// When `true`, run the END-TO-END daemon self-test (bind + accept loop +
    /// per-client tasks + real `daemon_loop`: a client attaches, submits, observes
    /// the resulting delta, then quits the daemon) then exit (`--daemon-selftest`).
    pub daemon_selftest: bool,
    /// When `true`, run the headless koma-daemon event loop with no terminal
    /// (`--daemon` flag). Owns the agent runtime; a TUI attaches as a client.
    pub daemon: bool,
    /// When `true`, run the GLOBAL MCP daemon with no terminal (`--mcp-daemon` flag):
    /// a singleton process that owns every configured MCP server connection so
    /// session-daemons can proxy to it (`~/.koma/mcp.sock`) instead of each spawning
    /// their own copies. No `--session` — it is not keyed to any session. Spawned by
    /// `ensure_mcp_daemon_running`; consumed by the session-daemon MCP proxy in the
    /// next commit.
    pub mcp_daemon: bool,
    /// When `true`, run as a thin client that attaches to a running daemon
    /// (`--attach` flag): connect to `~/.koma/daemon.sock`, render the daemon's
    /// foreground session from streamed snapshots/deltas, and forward input.
    pub attach: bool,
    /// When `true`, force the OLD standalone local TUI (`--local` flag) — the escape
    /// hatch from the daemon-by-default launch. `main` refuses this if a daemon is
    /// already alive (running a second writer against the daemon's sessions would
    /// corrupt the session locks); with no daemon up it runs the fully-standalone TUI.
    pub local: bool,
    /// A `koma daemon …` management invocation, if one was given (#118).
    /// `Some(Run(sub))` short-circuits `main` into that management action; `Some(Usage)`
    /// short-circuits into a usage print + non-zero exit (bare/unknown verb); `None`
    /// is the normal path (TUI / other flags). Either `Some` exits before the TUI.
    pub subcommand: Option<DaemonCli>,
    /// A `koma ext …` invocation, if one was given: the dev-sideload verb family
    /// (`ext install --dev <path>`). Mirrors `subcommand`'s short-circuit shape —
    /// `Some(_)` (valid or [`ExtCli::Usage`]) diverts `main` before the TUI starts.
    pub ext: Option<ExtCli>,
    /// When `true`, stop the running daemon then run the installer to fetch the
    /// latest release binary, then exit (`koma update` positional verb).
    pub update: bool,
    /// The session UUID this invocation is bound to (`--session <id>`).
    ///
    /// Daemon-per-session: a fresh `koma` MINTS this on the CLIENT side, then spawns its
    /// daemon with `--daemon --session <id>` and connects to the matching keyed socket
    /// (`run/<id>.sock`). On the DAEMON side it is REQUIRED — the daemon binds that
    /// socket and create-or-loads exactly session `<id>`. `None` on a bare `koma` until
    /// `main` mints one; a stray `--session` on a path that ignores it is harmless.
    pub session: Option<String>,
    /// When `true`, launch the feature-gated desktop GUI (`koma gui` positional verb):
    /// a wry webview hosting xterm.js that renders the real koma client in a PTY.
    pub gui: bool,
    /// When `true`, run the flutter-doctor-style readiness report (`koma doctor`
    /// positional verb) then exit. Read-only — never spawns/restarts/installs/kills
    /// anything.
    pub doctor: bool,
    /// Modifier for `doctor`: `-v`/`--verbose` — always show every category's
    /// sub-detail lines, not just the ones for [!]/[✗] categories.
    pub doctor_verbose: bool,
}

/// Print `koma <version>` to STDOUT and return the process exit code (`0`).
///
/// Handles `--version`/`-V` (#75). Unlike [`crate::app::print_daemon_usage`] this is
/// requested output, not error usage, so it goes to STDOUT with a zero exit code.
pub fn print_version() -> i32 {
    println!("koma {}", env!("CARGO_PKG_VERSION"));
    0
}

/// Print the user-facing CLI usage block to STDOUT and return the process exit
/// code (`0`).
///
/// Handles `--help`/`-h` (#75). Mirrors the module-level doc comment above (the
/// "User-facing surface" / "Positional verbs" sections) — only the advertised
/// surface, NOT the hidden plumbing flags (`--local`, `--daemon`, `--attach`,
/// `--ipc-selftest`, `--daemon-selftest`, `--mcp-daemon`).
pub fn print_help() -> i32 {
    println!(
        "koma — an agentic coding TUI\n\
         \n\
         usage: koma [command] [flags]\n\
         \n\
         commands:\n\
         \x20 (none)                        default: spawn-or-attach the daemon, then run the client\n\
         \x20 agents                         open the session hub (alias for --resume)\n\
         \x20 alone                          standalone no-daemon TUI (alias for --local)\n\
         \x20 gui                            launch the desktop GUI client\n\
         \x20 update                         stop the daemon, fetch the latest release, then exit\n\
         \x20 daemon <status|kill|restart|clean>  daemon management CLI\n\
         \x20 ext install --dev <zip|dir>    sideload an unsigned local extension (dev-only)\n\
         \x20 doctor [-v|--verbose]          readiness report (config/daemons/models/gui/…)\n\
         \n\
         flags:\n\
         \x20 --resume                       open the session hub\n\
         \x20 --session <id>                 bind this invocation to a specific session id\n\
         \x20 --version, -V                  print the version and exit\n\
         \x20 --help, -h                     print this help and exit"
    );
    0
}

/// Parse command-line arguments into [`Opts`].
///
/// All flags may appear anywhere in the argument list; position is not
/// significant. Unknown flags are silently ignored.
///
/// The positional VERBS are read from the first non-`--` argument (after `argv[0]`):
/// - `agents` sets `resume` (alias for `--resume`).
/// - `alone` sets `local` (alias for `--local`).
/// - `daemon` makes this a daemon-CLI invocation; the following non-`--` argument is
///   its verb — a valid one yields `Some(DaemonCli::Run(sub))`, a missing/unrecognised
///   one yields `Some(DaemonCli::Usage)`. EITHER short-circuits the TUI in `main`, so a
///   typo like `koma daemon staus` prints usage instead of silently opening the terminal.
///
/// Because `agents`/`alone` only set a bool the equivalent flag already sets, `main`'s
/// routing is unchanged — the verbs reuse the same resume / `--local` (guarded) paths.
pub fn parse(args: impl IntoIterator<Item = String>) -> Opts {
    let mut opts = Opts::default();

    // Collect once so we can both flag-match and positionally scan for the
    // `daemon <verb>` subcommand. `argv[0]` (the program path) is skipped for the
    // positional scan so a binary literally named `daemon` can't be mistaken for the
    // subcommand, while still being flag-matched (it lands in the `_` arm, a no-op).
    let all: Vec<String> = args.into_iter().collect();

    for arg in &all {
        match arg.as_str() {
            "--resume"                       => opts.resume = true,
            "--internet-fullmode-install"    => opts.internet_fullmode_install = true,
            "--internet-fullmode-uninstall"  => opts.internet_fullmode_uninstall = true,
            "--security-install"             => opts.security_install = true,
            "--force"                        => opts.force = true,
            "--ipc-selftest"                 => opts.ipc_selftest = true,
            "--daemon-selftest"              => opts.daemon_selftest = true,
            "--daemon"                       => opts.daemon = true,
            "--mcp-daemon"                   => opts.mcp_daemon = true,
            "--attach"                       => opts.attach = true,
            "--local"                        => opts.local = true,
            _                                => {}
        }
    }

    // `--session <id>` takes a VALUE, so it can't be handled in the value-less match
    // above. Scan consecutive pairs and capture the token AFTER `--session` (last one
    // wins). The value is not validated here — the daemon/client treat it as an opaque
    // socket key; a missing value (trailing `--session`) leaves `session = None`.
    for pair in all.windows(2) {
        if pair[0] == "--session" {
            opts.session = Some(pair[1].clone());
        }
    }

    // Positional verb/subcommand scan: skip argv[0] and look at the FIRST positional
    // (non-flag) token. It can be one of the friendly verbs that front the plumbing
    // flags, or the `daemon` management subcommand:
    //
    // - `agents` — alias for `--resume`: open the session hub. Sets the SAME
    //   `opts.resume` bit the flag does, so `main` reuses the identical resume path
    //   (no duplicated routing).
    // - `alone`  — alias for `--local`: standalone no-daemon TUI. Sets the SAME
    //   `opts.local` bit the flag does, so `main` reuses the identical `--local`
    //   branch INCLUDING its daemon-alive guard (the guard is not re-implemented here).
    // - `daemon <verb>` — management subcommand: read the next positional as the verb.
    //   A bare `daemon` (no following positional) or an unknown verb maps to
    //   `DaemonCli::Usage` so `main` prints usage instead of dropping into the TUI.
    //
    // Only the FIRST positional selects a verb; `agents`/`alone` consume no further
    // positionals, so they cannot collide with the `daemon <verb>` parsing.
    let mut positional = all.iter().skip(1).filter(|a| !a.starts_with("--"));
    match positional.next().map(String::as_str) {
        Some("agents") => opts.resume = true,
        Some("alone") => opts.local = true,
        Some("update") => opts.update = true,
        Some("gui") => opts.gui = true,
        Some("doctor") => {
            opts.doctor = true;
            opts.doctor_verbose = all.iter().any(|a| a == "-v" || a == "--verbose");
        }
        Some("daemon") => {
            opts.subcommand = Some(match positional.next().and_then(|v| DaemonSub::from_verb(v)) {
                Some(sub) => DaemonCli::Run(sub),
                None => DaemonCli::Usage,
            });
        }
        Some("ext") => {
            // `ext install --dev <path>` is the only real verb; `--dev`'s value is
            // NOT a positional (the loop above only iterates non-`--` tokens), so it
            // is read the same way `--session <id>` is: scan consecutive pairs in
            // `all` for the token right after `--dev`.
            opts.ext = Some(match positional.next().map(String::as_str) {
                Some("install") => {
                    let dev_path = all
                        .windows(2)
                        .find(|pair| pair[0] == "--dev")
                        .map(|pair| pair[1].clone());
                    match dev_path {
                        Some(path) => ExtCli::InstallDev(path),
                        None => ExtCli::Usage,
                    }
                }
                _ => ExtCli::Usage,
            });
        }
        _ => {}
    }

    opts
}
