//! The `koma daemon <verb>` subcommand bodies (`status`/`kill`/`restart`/`clean`)
//! plus their shared stale-file sweep helpers. Split out of [`super`] (the
//! `manage` module) for file size — pure code motion, no behaviour change.
//!
//! `print_daemon_usage` is re-exported from `manage`
//! (`pub use commands::print_daemon_usage;`) so the existing
//! `crate::app::runtime::manage::print_daemon_usage` re-export chain (through
//! `runtime`/`app`, consumed by `main`) keeps resolving unchanged. `cmd_status`/
//! `cmd_kill`/`cmd_restart`/`cmd_clean` are bumped to `pub(super)` (were private) —
//! called from `super::run_daemon_subcommand`.

use std::path::Path;

use anyhow::{anyhow, Result};

use crate::ipc::proto::{ClientRequest, DaemonEvent};
use crate::model::store;

/// Print usage for the `daemon` subcommand (bare/unknown verb). Returns the process
/// exit code the caller should use (non-zero — a malformed invocation is an error).
pub fn print_daemon_usage() -> i32 {
    eprintln!(
        "usage: koma daemon <status|kill|restart|clean|delete>\n\
         \n\
         \x20 status                show whether the koma daemon is running (PID, socket, sessions)\n\
         \x20 kill                  stop every live session-daemon (escalates to signals if needed)\n\
         \x20 kill --session <id>   stop only the session-daemon for <id>\n\
         \x20 delete --session <id> physically delete one on-disk history session (refuses if live)\n\
         \x20 restart               stop the daemon (if any) then start a fresh one\n\
         \x20 clean                 remove a stale socket/pidfile when NO daemon is running"
    );
    2
}

/// `koma daemon status` — report liveness for EVERY session-daemon via the bind-as-oracle
/// probe.
///
/// Daemon-per-session: enumerates the live `run/<id>.sock` daemons and prints one block
/// per session — its session id, advisory PID (from that session's pidfile), socket
/// path, and a best-effort session count from the daemon's own `ListSessions` snapshot
/// (failure to get the count never fails the command — liveness is already established).
/// With no live daemons it prints "no daemons running".
pub(super) fn cmd_status() -> Result<()> {
    let live = super::live_session_sockets()?;
    let mcp_live = super::mcp::mcp_daemon_alive();
    let oauth_live = super::oauth::oauth_daemon_alive();

    if live.is_empty() && !mcp_live && !oauth_live {
        println!("koma daemon: no daemons running");
        return Ok(());
    }

    if live.is_empty() {
        println!("koma daemon: no session daemons running");
    } else {
        println!("koma daemon: {} session daemon(s) running", live.len());
    }
    for (id, sock) in live {
        // PID is advisory (the pidfile may be missing/stale even while the socket is up —
        // they are written/removed at slightly different moments), so word it as such.
        let pid_str = match super::read_pidfile(&id) {
            Some(pid) => format!("pid {pid}"),
            None => "pid unknown (no pidfile)".to_string(),
        };
        println!("  session {id} ({pid_str})");
        println!("    socket: {}", sock.display());

        // Best-effort session count from THIS daemon's snapshot. (At this commit a
        // daemon owns exactly one session, so this is normally 1; kept because the wire
        // reply still carries the full set.) Any failure is non-fatal.
        match daemon_session_count(&sock) {
            Ok(n) => println!("    sessions: {n}"),
            Err(e) => println!("    sessions: unknown ({e})"),
        }
    }

    // Report the GLOBAL MCP daemon (singleton) too, so it is manageable/visible.
    if mcp_live {
        let pid_str = match super::mcp::read_mcp_pidfile() {
            Some(pid) => format!("pid {pid}"),
            None => "pid unknown (no pidfile)".to_string(),
        };
        println!("  MCP daemon ({pid_str})");
        if let Ok(sock) = store::mcp_daemon_sock_path() {
            println!("    socket: {}", sock.display());
        }
    }

    // Report the GLOBAL OAuth daemon (singleton) too.
    if oauth_live {
        let pid_str = match super::oauth::read_oauth_pidfile() {
            Some(pid) => format!("pid {pid}"),
            None => "pid unknown (no pidfile)".to_string(),
        };
        println!("  OAuth daemon ({pid_str})");
        if let Ok(sock) = store::oauth_daemon_sock_path() {
            println!("    socket: {}", sock.display());
        }
    }

    Ok(())
}

/// Ask the live daemon for its session count via `ListSessions` → `Snapshot`.
///
/// Bounded: it reads at most a handful of frames (skipping any interleaved Ack/Error)
/// before giving up, and the socket's read timeout caps the wait, so a daemon that
/// never answers surfaces as an `Err` (rendered as "unknown") rather than a hang.
fn daemon_session_count(sock: &Path) -> Result<usize> {
    let (mut stream, mut reader) = super::connect_managed(sock)?;
    super::send_request(&mut stream, &ClientRequest::ListSessions)?;

    // The reply we want is a Snapshot; tolerate a few non-Snapshot frames first.
    for _ in 0..8 {
        let frame = super::recv_frame(&mut stream, &mut reader)?;
        if let DaemonEvent::Snapshot(snap) = frame.event {
            return Ok(snap.sessions.len());
        }
    }
    Err(anyhow!("no snapshot in reply"))
}

/// `koma daemon delete --session <id>` — physically delete one on-disk history session.
///
/// Used by the remote hub HISTORY pane over SSH. Refuses when the session is live
/// (socket accepting) so a cooking row can never be wiped out from under its daemon.
/// Missing id is best-effort success (already gone) so delete is idempotent.
pub(super) fn cmd_delete(session: Option<&str>) -> Result<()> {
    let id = session.ok_or_else(|| anyhow!("delete requires --session <id>"))?;
    if id.is_empty() || id.contains('\0') {
        anyhow::bail!("invalid session id");
    }
    // Refuse live — remote HISTORY should never include live ids, but re-check here.
    if super::daemon_alive(id) {
        anyhow::bail!("refusing to delete live session {id}");
    }
    // Also refuse if any live list still reports it (belt + suspenders vs bind race).
    if super::list_live_sessions()
        .into_iter()
        .any(|s| s.session_id == id)
    {
        anyhow::bail!("refusing to delete live session {id}");
    }
    let metas = store::list_all_sessions()?;
    if let Some(meta) = metas.into_iter().find(|m| m.id == id) {
        if meta.locked {
            anyhow::bail!("refusing to delete locked session {id}");
        }
        // Final TOCTOU re-probe immediately before the remove.
        if super::daemon_alive(id) {
            anyhow::bail!("refusing to delete live session {id}");
        }
        store::delete_session(&meta.path)?;
        println!("koma daemon: deleted session {id}");
    } else {
        // Already gone — best-effort success (mirrors kill missing-id).
        println!("koma daemon: session {id} not found (already gone)");
    }
    Ok(())
}

/// `koma daemon kill` — stop EVERY live session-daemon, escalating per session only if
/// one won't go.
///
/// Daemon-per-session: enumerates the live `run/<id>.sock` daemons and calls
/// [`super::stop_session_daemon`] on each (which prints its own per-session outcome). Each stop
/// is best-effort — one wedged session never blocks stopping the rest. A run dir with no
/// live daemons reports "no daemons running" (and still sweeps any stale turds).
///
/// Also stops the GLOBAL MCP daemon (best-effort), then
/// runs [`super::os::kill_orphan_daemon_processes`] — a `/proc` sweep for koma daemon
/// processes the socket scan structurally can't see (socket file removed out from under
/// a still-running daemon, or a daemon spawned by an older/different-path binary). That
/// keeps "no daemons running" honest and makes `kill` reliably clear the way for a
/// reinstall (a lingering orphan otherwise holds the binary, causing "Text file busy").
pub(super) fn cmd_kill(session: Option<&str>) -> Result<()> {
    // `koma daemon kill --session <id>`: stop exactly one session-daemon (remote hub
    // kill over SSH uses this). Leave MCP/OAuth/linker alone — those are host-global.
    if let Some(id) = session {
        if id.is_empty() || id.contains('\0') {
            anyhow::bail!("invalid session id");
        }
        let _ = super::stop_session_daemon(id, false);
        return Ok(());
    }

    let live = super::live_session_sockets()?;
    let mcp_live = super::mcp::mcp_daemon_alive();

    if live.is_empty() && !mcp_live {
        // Nothing visible via the socket scan — but an orphan daemon (socket removed,
        // or spawned by an older/different-path binary) may still be running. Sweep
        // before declaring victory.
        let orphans = super::os::kill_orphan_daemon_processes();
        if orphans > 0 {
            println!("koma daemon: killed {orphans} orphan daemon process(es)");
        } else {
            println!("koma daemon: no daemons running");
        }
        // Sweep any stale socket/pidfiles left by crashed daemons.
        sweep_stale_files();
        super::mcp::unlink_mcp_daemon_files();
        super::oauth::unlink_oauth_daemon_files();
        return Ok(());
    }
    for (id, _path) in live {
        let _ = super::stop_session_daemon(&id, false);
    }
    // Stop the GLOBAL MCP daemon too (best-effort; prints its own outcome). Only bother
    // when it's live — a dead one just gets its stale files swept below via its own
    // not-running path.
    if mcp_live {
        // `koma daemon kill` owns a terminal — print the outcome (not quiet).
        super::mcp::stop_mcp_daemon(false);
    }
    // Stop the GLOBAL OAuth daemon too (best-effort). Same pattern as MCP.
    if super::oauth::oauth_daemon_alive() {
        super::oauth::stop_oauth_daemon(false);
    }
    // Stop the GLOBAL Linker daemon too (best-effort). Same pattern as MCP/OAuth.
    #[cfg(feature = "linker")]
    if super::linker::linker_daemon_alive() {
        super::linker::stop_linker_daemon(false);
    }
    // Catch any socket-less orphans the scan above couldn't see, regardless of whether
    // any keyed sockets were found live.
    let orphans = super::os::kill_orphan_daemon_processes();
    if orphans > 0 {
        println!("koma daemon: killed {orphans} additional orphan daemon process(es)");
    }
    sweep_stale_files();
    Ok(())
}

/// `koma daemon restart` — stop EVERY live session-daemon, then respawn one per session
/// (each on its own keyed socket) and report the new PIDs.
///
/// Reuses [`super::restart_daemon`] (the per-session graceful→signal stop + spawn-and-confirm)
/// for each currently-live session, so "restart" is "a working daemon is up afterwards
/// for every session that was running", not just "children were forked". A restart error
/// for one session is surfaced but never blocks the others. With nothing live there is
/// nothing to restart (a fresh `koma` is how you start a daemon — restart only re-spawns
/// sessions that were already running).
pub(super) fn cmd_restart() -> Result<()> {
    let live = super::live_session_sockets()?;
    if live.is_empty() {
        println!("koma daemon: no daemons running to restart (start one with `koma`)");
        return Ok(());
    }
    for (id, _path) in live {
        if let Err(e) = super::restart_daemon(&id, false) {
            eprintln!("koma daemon: failed to restart session {id}: {e:#}");
        }
    }
    Ok(())
}

/// `koma daemon clean` — the "OS shit happened, nuke the turds" escape hatch.
///
/// Daemon-per-session: scans every `run/<id>.sock`. For each socket whose daemon is DEAD
/// (bind-as-oracle probe refused) it unlinks that socket + its pidfile; sockets with a
/// LIVE daemon are left untouched (removing a live daemon's socket would orphan it). Then
/// it also sweeps any orphan `*.pid` whose `*.sock` is already gone. Reports exactly
/// which files it removed, and — when some daemons are still live — names them so the
/// user can `koma daemon kill` instead.
pub(super) fn cmd_clean() -> Result<()> {
    let socks = super::list_session_sockets()?;
    let live: Vec<String> = socks
        .iter()
        .filter(|(_, _, alive)| *alive)
        .map(|(id, _, _)| id.clone())
        .collect();

    let mut removed: Vec<String> = Vec::new();
    // Remove every DEAD session's socket + pidfile; never touch a live one.
    for (id, path, alive) in &socks {
        if *alive {
            continue;
        }
        // Unix: the socket is a real file under run_dir — unlink it. Windows: a
        // named pipe is not a filesystem object (it vanishes with its last handle,
        // and `path` here is a pipe-namespace path, not a run_dir file), so there is
        // no stale socket FILE to remove — only the pidfile below applies there.
        #[cfg(unix)]
        if std::fs::remove_file(path).is_ok() {
            removed.push(path.display().to_string());
        }
        #[cfg(windows)]
        let _ = path;
        if let Ok(pid) = store::daemon_pid_path(id) {
            if std::fs::remove_file(&pid).is_ok() {
                removed.push(pid.display().to_string());
            }
        }
    }

    // Sweep orphan pidfiles whose socket is already gone (a crash that lost the socket
    // but left the pid). Skip any pid that belongs to a still-live session.
    for (id, pid) in orphan_pidfiles()? {
        if live.contains(&id) {
            continue;
        }
        if std::fs::remove_file(&pid).is_ok() {
            removed.push(pid.display().to_string());
        }
    }

    // GLOBAL MCP daemon: only clean its files when it is DEAD (bind-as-oracle refused);
    // a live one is left untouched (removing its socket would orphan it). It has no
    // orphan-pid sweep of its own — the socket+pid pair is nuked together when dead.
    let mcp_live = super::mcp::mcp_daemon_alive();
    if !mcp_live {
        if let Ok(sock) = store::mcp_daemon_sock_path() {
            if std::fs::remove_file(&sock).is_ok() {
                removed.push(sock.display().to_string());
            }
        }
        if let Ok(pid) = store::mcp_daemon_pid_path() {
            if std::fs::remove_file(&pid).is_ok() {
                removed.push(pid.display().to_string());
            }
        }
    }

    // GLOBAL OAuth daemon: same cleanup as MCP.
    if !super::oauth::oauth_daemon_alive() {
        if let Ok(sock) = store::oauth_daemon_sock_path() {
            if std::fs::remove_file(&sock).is_ok() {
                removed.push(sock.display().to_string());
            }
        }
        if let Ok(pid) = store::oauth_daemon_pid_path() {
            if std::fs::remove_file(&pid).is_ok() {
                removed.push(pid.display().to_string());
            }
        }
    }

    if !live.is_empty() {
        println!(
            "koma daemon: {} session daemon(s) still running ({}); left their files in place — \
             use `koma daemon kill` to stop them",
            live.len(),
            live.join(", ")
        );
    }
    if mcp_live {
        println!(
            "koma daemon: MCP daemon still running; left its files in place — \
             use `koma daemon kill` to stop it"
        );
    }
    if super::oauth::oauth_daemon_alive() {
        println!(
            "koma daemon: OAuth daemon still running; left its files in place — \
             use `koma daemon kill` to stop it"
        );
    }
    if removed.is_empty() {
        println!("koma daemon: nothing to clean (no stale socket/pidfile)");
    } else {
        println!("koma daemon: removed stale file(s):");
        for f in removed {
            println!("  {f}");
        }
    }
    Ok(())
}

/// Best-effort sweep of stale socket/pidfiles for sessions whose daemon is DEAD, across
/// the whole run dir. Used by `cmd_kill`'s no-live-daemons branch to clean crash turds.
/// Live daemons are never touched. Errors are swallowed (pure cleanup).
///
/// `pub(super)` — also called from `super::list_live_sessions`.
pub(super) fn sweep_stale_files() {
    if let Ok(socks) = super::list_session_sockets() {
        for (id, path, alive) in socks {
            if alive {
                continue;
            }
            // Unix: unlink the stale socket file. Windows: no stale socket FILE
            // exists to remove (see the `cmd_clean` comment above) — just drop the
            // (pipe-namespace) path unused.
            #[cfg(unix)]
            let _ = std::fs::remove_file(&path);
            #[cfg(windows)]
            let _ = path;
            if let Ok(pid) = store::daemon_pid_path(&id) {
                let _ = std::fs::remove_file(pid);
            }
        }
    }
    if let Ok(orphans) = orphan_pidfiles() {
        for (_id, pid) in orphans {
            let _ = std::fs::remove_file(pid);
        }
    }
}

/// Enumerate `*.pid` files in the run dir whose matching `*.sock` does NOT exist (an
/// orphan pidfile from a crash that lost its socket), as `(session_id, pid_path)` pairs.
/// Best-effort: an unreadable run dir yields an empty list.
fn orphan_pidfiles() -> Result<Vec<(String, std::path::PathBuf)>> {
    let dir = store::run_dir()?;
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(out),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("pid") {
            continue;
        }
        let Some(id) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        // Only an orphan if there is no corresponding socket endpoint.
        if sock_gone_for(id) {
            out.push((id.to_string(), path));
        }
    }
    Ok(out)
}

/// Whether `id`'s session-daemon endpoint no longer exists — the "is this pidfile
/// an orphan" half of [`orphan_pidfiles`] (NOT a liveness check: a present-but-dead
/// endpoint is still "not gone" here; [`super::list_session_sockets`]'s
/// connect-probe is what decides liveness elsewhere).
///
/// Unix: the socket is a real file under run_dir, so a plain `exists()` answers it.
/// Windows: `daemon_sock_path(id).exists()` would lie — a named pipe is not a
/// filesystem object, so `exists()` on its pipe-namespace path is meaningless here
/// — the real answer is whether `koma-<id>` currently appears in the pipe
/// namespace, via [`store::list_koma_session_pipes`].
#[cfg(unix)]
fn sock_gone_for(id: &str) -> bool {
    store::daemon_sock_path(id)
        .map(|s| !s.exists())
        .unwrap_or(true)
}

#[cfg(windows)]
fn sock_gone_for(id: &str) -> bool {
    !store::list_koma_session_pipes()
        .iter()
        .any(|pipe_id| pipe_id == id)
}
