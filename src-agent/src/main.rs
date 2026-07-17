//! Binary entry point for the koma TUI.
//!
//! Parses CLI arguments via [`cli::parse`], handles any short-circuit modes
//! (provisioner flags), then routes into one of the launch paths below.
//!
//! # Launch routing (Stage 7: daemon-by-default)
//!
//! User-facing surface (what `--help` advertises):
//!
//! | Invocation | Action |
//! |---|---|
//! | `koma` (no args) | **default** — ensure a daemon is running (spawn a detached daemon if none is up), then attach as a thin client. NO fallback to local. |
//! | `koma agents` | open the session hub (alias for `--resume`). |
//! | `koma --resume` | open the session hub. |
//! | `koma alone` | standalone no-daemon TUI ([`app::run`]); REFUSES if a daemon is already alive. The escape hatch (alias for `--local`). |
//! | `koma daemon <status\|kill\|restart\|clean>` | daemon management CLI then exit. |
//! | `koma ext install --dev <zip\|dir>` | sideload an unsigned local extension (offline, no koma.run account) then exit. |
//! | `koma gui` | feature-gated (`--features gui`) desktop client — a wry webview hosting xterm.js that renders the real koma terminal client spawned in a PTY. |
//! | `koma --internet-fullmode-install [--force]` | provision Python full-mode (browser) env then exit. |
//! | `koma --internet-fullmode-uninstall` | remove Python full-mode env then exit. |
//!
//! Hidden plumbing (still parsed + functional, just not advertised — the verbs above
//! front them, and the default/management paths drive them internally):
//!
//! | Flag | Action |
//! |---|---|
//! | `--local` | fronted by `koma alone`: force the OLD standalone local TUI; REFUSES if a daemon is already alive. |
//! | `--daemon` | run the headless koma-daemon event loop (no TUI). The default path execs this to spawn the daemon. |
//! | `--attach` | attach to an ALREADY-running daemon as a thin client (does not spawn one). |
//! | `--ipc-selftest` | round-trip the daemon IPC transport then exit. |
//! | `--daemon-selftest` | drive the full daemon stack end-to-end then exit. |
//!
//! Data flow overview:
//! ```text
//! terminal event
//!     → controller::input::handle_key  (KeyEvent → Action)
//!     → app::runtime (Action → state mutation + optional async API call)
//!     → view::draw   (AppState → rendered Frame)
//! ```

mod app;
mod cli;
mod config;
mod controller;
mod dto;
mod internet;
mod ipc;
mod security;
mod model;
mod resources;
mod service;
mod tool;
mod view;

fn main() -> anyhow::Result<()> {
    // --- short-circuit: `--version`/`-V`/`--help`/`-h` (#75) ---
    // Must run BEFORE any side effect (legacy-dir migration, catalogue overlay init,
    // daemon spawn) — these just print and exit, they must not touch disk or state.
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--version" | "-V" => std::process::exit(cli::print_version()),
            "--help" | "-h" => std::process::exit(cli::print_help()),
            _ => {}
        }
    }

    // Migrate legacy config dir (~/.simple-coder -> ~/.koma) before anything
    // reads base_dir(), so every entry path (TUI, --internet-fullmode-install,
    // --resume) sees the migrated directory.
    model::store::migrate_legacy_dir();

    // Seed the catalogue overlay (reasoning/pricing metadata for non-OpenRouter
    // providers) and kick off its non-blocking background refresh. Runs here,
    // before the CLI subcommand fork, so both `koma` (TUI/daemon) and `koma gui`
    // (which execs a bare `koma` client) hit it exactly once at startup.
    service::catalogue_overlay::init();

    let opts = cli::parse(std::env::args());

    // --- short-circuit: `koma daemon <verb>` management CLI (no TUI) ---
    // Mirrors how the provisioner / self-test flags short-circuit BEFORE the TUI, but
    // this is a positional SUBCOMMAND (status/kill/restart/clean) — the operator
    // control surface for the headless daemon (#118). It must work even when the TUI
    // can't start, so it runs first and exits the process directly. A bare/unknown verb
    // (`DaemonCli::Usage`) prints usage and exits non-zero. The default launch path is
    // unchanged — `daemon` is the only token that diverts here.
    if let Some(sub) = opts.subcommand {
        match sub {
            cli::DaemonCli::Run(verb) => {
                return match app::run_daemon_subcommand(verb) {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        eprintln!("error: {e:#}");
                        std::process::exit(1);
                    }
                };
            }
            cli::DaemonCli::Usage => {
                std::process::exit(app::print_daemon_usage());
            }
        }
    }

    // --- short-circuit: `koma ext install --dev <path>` (no TUI, no daemon) ---
    // Pre-daemon, offline sideload: verify+unpack an unsigned local zip/dir straight
    // into `~/.koma/extensions/<id>/` and the config registry. Mirrors the `daemon`
    // subcommand short-circuit above — must work even when the TUI can't start.
    if let Some(sub) = opts.ext.clone() {
        return match sub {
            cli::ExtCli::InstallDev(path) => match app::run_ext_install_dev(&path) {
                Ok(()) => Ok(()),
                Err(e) => {
                    eprintln!("error: {e:#}");
                    std::process::exit(1);
                }
            },
            cli::ExtCli::Usage => {
                std::process::exit(app::print_ext_usage());
            }
        };
    }

    // --- upgrade migration: reap any pre-0.2.0 global daemon on first 0.2.0 launch ---
    // The old 0.1.x daemon bound a bare `<base_dir>/daemon.sock`. 0.2.0 never creates
    // that path; its presence means a pre-0.2.0 orphan is still running. We SIGTERM it
    // here (best-effort) so it releases any session write-locks it holds and vacates disk.
    // Skip in the daemon/mcp-daemon children — they are spawned AFTER this migration runs
    // in the parent, and a child re-running migrate would be a no-op race anyway.
    if !opts.daemon && !opts.mcp_daemon {
        app::migrate_legacy_daemon();
    }

    // --- short-circuit: `koma update` — stop daemon + run installer (no TUI) ---
    if opts.update {
        return match crate::app::run_update() {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("koma update failed: {e}");
                std::process::exit(1);
            }
        };
    }

    // --- short-circuit: provisioner modes (no TUI) ---

    if opts.internet_fullmode_install {
        return match internet::install(opts.force) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        };
    }

    if opts.internet_fullmode_uninstall {
        return match internet::uninstall() {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        };
    }

    if opts.security_install {
        return match security::install(opts.force) {
            Ok(()) => Ok(()),
            Err(e) => {
                eprintln!("error: {e:#}");
                std::process::exit(1);
            }
        };
    }

    // --- short-circuit: IPC transport self-test (no TUI) ---
    // Exercises frame.rs + server.rs + client.rs end-to-end; never returns
    // (always exits the process with OK/FAIL status).
    if opts.ipc_selftest {
        ipc::selftest::run();
    }

    // --- short-circuit: END-TO-END daemon self-test (no TUI) ---
    // Drives the full stage-5 stack (bind + accept loop + per-client tasks + the
    // real daemon_loop hub) over a real socket: a client attaches, submits, sees
    // the resulting delta, then quits the daemon. Never returns (OK/FAIL exit).
    if opts.daemon_selftest {
        app::run_daemon_selftest();
    }

    // --- headless path: run the GLOBAL MCP daemon (no TUI) ---
    // A singleton process that owns every configured MCP server connection so
    // session-daemons proxy to it (`~/.koma/mcp.sock`) instead of each spawning their
    // own copies of a heavyweight server (e.g. `serena`). No `--session`; it is not
    // keyed to any session. Persists until signalled. Checked BEFORE `--daemon` so a
    // stray combination can't accidentally take the session-daemon branch.
    if opts.mcp_daemon {
        return app::run_mcp_daemon(opts);
    }

    // --- headless path: run the koma-daemon event loop (no TUI) ---
    // Owns the agent runtime with no terminal; a TUI attaches as a thin client via
    // `--attach`. Stays in this branch (loops forever) until QuitDaemon / Ctrl-C.
    if opts.daemon {
        return app::run_daemon(opts);
    }

    // --- desktop GUI path: feature-gated wry + xterm.js client (spawns the real
    // koma terminal client in a PTY). Default builds omit the `gui` feature. ---
    #[cfg(feature = "gui")]
    if opts.gui {
        return app::run_gui(opts);
    }
    #[cfg(not(feature = "gui"))]
    if opts.gui {
        eprintln!("koma was built without the `gui` feature. Rebuild with: cargo build --features gui");
        std::process::exit(1);
    }

    // --- explicit thin-client path: attach to an ALREADY-running daemon ---
    // Daemon-per-session: connects to the keyed socket `run/<id>.sock` of the session
    // named by `--session <id>` (REQUIRED here — there is no longer a single global
    // socket to default to), renders that daemon's foreground session from streamed
    // snapshots/deltas, and forwards input. Detaching (Ctrl-C) leaves the daemon running.
    // Unlike the default path below, `--attach` does NOT spawn a daemon: a missing daemon
    // (or a missing `--session`) surfaces as an error (the operator asked to attach to one
    // that should already exist).
    if opts.attach {
        return app::client_run(opts);
    }

    // --- escape hatch: force the OLD standalone local TUI (`--local`) ---
    // The fully-standalone single-process TUI that owns its own session runtime — the
    // way `koma` launched before the daemon-by-default flip (Stage 7). GUARDED: if a
    // daemon is ALREADY alive (bind-as-oracle probe), refuse, because a local TUI is a
    // SECOND writer against the same on-disk sessions/locks and dual-PID ownership
    // corrupts the locks. Direct the user to attach (plain `koma`) or kill the daemon
    // first, and exit non-zero. With no daemon up, run the standalone TUI normally.
    if opts.local {
        if app::any_daemon_alive() {
            eprintln!(
                "error: a koma daemon is running; use `koma` to attach to it, \
                 or `koma daemon kill` first (refusing to run a standalone local TUI \
                 against a live daemon — it would corrupt the session locks)"
            );
            std::process::exit(1);
        }
        return app::run(opts);
    }

    // --- DEFAULT: ensure a daemon is running, then attach as a thin client ---
    // This is THE daemon-by-default launch: `koma` with no flags first guarantees a
    // daemon is accepting (connect if one is up, else spawn a DETACHED `koma --daemon`
    // and poll-connect until it accepts — the Stage-13 spawn-or-attach machinery), then
    // hands off to the SAME thin client `--attach` uses (`client_run` opens its own
    // connection to the now-live daemon and renders its foreground session).
    //
    // NO AUTO-FALLBACK TO LOCAL: if the daemon can't be spawned/connected, surface a
    // CLEAR error and exit — never silently drop into a standalone local TUI. A local
    // TUI here would be a second writer against the daemon's sessions (the dual-writer
    // corruption trap); `--local` is the explicit, guarded opt-in for standalone mode.
    //
    // Creds note: a freshly-spawned daemon whose session has no api_key sits in KeyInput
    // mode, which the client now renders (#122) — the user enters creds via the client,
    // forwarded to the daemon. So a first-ever `koma` (no prior creds) reaches a usable
    // KeyInput screen through the client, not a crash.
    //
    // Daemon-per-session: a fresh `koma` MINTS a new session UUID on the CLIENT side,
    // spawns its OWN daemon bound to `run/<id>.sock` via `--daemon --session <id>`, then
    // connects to that same keyed socket. Two `koma` in two terminals mint two different
    // ids → two fully independent daemons. The minted id is always new, so the daemon's
    // create-or-load always takes the CREATE branch here.
    //
    // `--resume` / `koma agents` is DIFFERENT: it opens the CLIENT-SIDE daemon swapper
    // FIRST and spawns/attaches NOTHING up front. We hand control straight to `client_run`
    // WITHOUT minting a session or spawning a daemon — seeing `opts.resume`, it starts in
    // its Swapper state, lists live + on-disk sessions, and only on a pick does it
    // ensure+attach the chosen session's daemon (minting a fresh id only for `[+ new
    // session]`). So a `--resume` launch never creates a stray empty session-daemon.
    if opts.resume {
        return app::client_run(opts);
    }

    let mut opts = opts;
    let session_id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = app::ensure_daemon_running(&session_id, false, None) {
        eprintln!("error: could not start the koma daemon: {e:#} — try `koma --local`");
        std::process::exit(1);
    }
    // Hand the minted id to the client so it connects to THIS session's keyed socket.
    opts.session = Some(session_id);
    app::client_run(opts)
}
