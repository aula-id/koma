//! Background-bash registry: run a shell command on a worker thread and poll it.
//!
//! Every model `bash` call (foreground or `run_in_background: true`) registers a
//! [`BashJob`] here. True background jobs set `tool_call_id: None` and return a
//! job id immediately; foreground jobs set `tool_call_id: Some(call.id)` and park
//! the turn on `pending_tool_tasks` until the child exits or the user promotes
//! with Ctrl+B (clears the call id → synthetic tool result; completion becomes a
//! nudge like true BG). Both poll via `bash_output` / stop via `bash_kill`.
//!
//! Concurrency shape mirrors the rest of the crate's off-thread work: a plain
//! `std::thread` owns the child wait (NOT a tokio task — the shell child must run
//! with no tokio runtime in context, same as the deferred lane), and the job's
//! mutable state lives behind an `Arc<`[`BashJobShared`]`>` shared between that
//! worker and the registry entry. Completion is signalled over an
//! `UnboundedSender<usize>` (the job id) so the event loop can deliver a tool
//! result (still-blocking FG) or a toast + nudge (true BG / promoted).

use std::io::{BufRead, BufReader};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use tokio::sync::mpsc::UnboundedSender;

/// Lifecycle state of a [`BashJob`], advanced by the worker thread (and flipped
/// to `Killed` by [`kill_bash_job`]). Mirrors the SHAPE of
/// [`crate::app::subagent::SubAgentStatus`] / [`crate::app::sec::SecStatus`].
///
/// - `Running`: the child is in flight (the initial state).
/// - `Done(code)`: the child exited; `code` is its exit status (`-1` if the
///   process was terminated by a signal and reported no code).
/// - `Killed`: terminated via [`kill_bash_job`] (`bash_kill`).
/// - `Error(msg)`: the child could not be spawned / waited on; `msg` is why.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum BashJobStatus {
    Running,
    Done(i32),
    Killed,
    Error(String),
}

/// The mutable state SHARED between the worker thread and the registry entry,
/// each field behind its own `Mutex` so the reader threads (appending output),
/// the worker (setting the terminal status), and the registry (snapshotting /
/// killing) never contend on one lock.
pub struct BashJobShared {
    /// Captured stdout+stderr, ANSI-stripped and capped to the last
    /// [`crate::config::MAX_TOOL_OUTPUT_CHARS`] chars (so a chatty long-running
    /// job can't grow this unbounded). Appended incrementally by the reader
    /// thread as the child emits output, so a `bash_output` poll sees progress.
    pub output: Mutex<String>,
    /// Current lifecycle state. Starts `Running`; the worker sets the terminal
    /// state on exit (unless already `Killed`).
    pub status: Mutex<BashJobStatus>,
    /// The child's OS pid, recorded the instant it is spawned so `bash_kill`
    /// can signal it. `None` until the child is spawned (or if the spawn failed).
    pub pid: Mutex<Option<u32>>,
    /// Wall-clock instant the job reached a terminal state (Done/Killed/Error);
    /// None while Running. Frozen so the /bash panel's elapsed timer stops at
    /// the final duration.
    pub ended_at: Mutex<Option<Instant>>,
    /// Absolute path of this job's tee'd full-output log, once written by
    /// [`BashJob::ensure_tee_log`]. `None` until the first qualifying
    /// `bash_output` poll actually needs it; idempotent thereafter — the SAME
    /// path is reused (never rewritten) once populated, so the model's
    /// full-output pointer for this job never changes out from under it.
    pub tee_path: Mutex<Option<std::path::PathBuf>>,
    /// Wall deadline for foreground-only timeout. `Some` while the job still
    /// blocks a main-turn tool call; cleared on Ctrl+B promote (or never set for
    /// true background). The worker polls this and times out only while set.
    pub deadline: Mutex<Option<Instant>>,
}

/// One registered background bash job: its identity, the command, when it
/// started, and the shared mutable state the worker thread updates.
pub struct BashJob {
    /// Stable per-session id, allocated from `SessionRuntime::next_bash_job_id`.
    /// Surfaced to the model as `bash-<id>`.
    pub id: usize,
    /// The shell command this job runs. Read by the `/bash` panel + chat-line
    /// rendering, and by the `bash_output` poll path (`render_finished_output`
    /// / `ensure_tee_log`) to apply the same command-aware output filter as
    /// synchronous `bash`/`git_operator`.
    pub command: String,
    /// Wall-clock instant the job was registered. Read by the `/bash` panel (a
    /// later stage) to show how long a job has been running.
    #[allow(dead_code)]
    pub started_at: Instant,
    /// When `Some`, this job is blocking a main-turn tool call (foreground bash).
    /// Cleared on Ctrl+B promote or when the Done path consumes it into
    /// `tool_results`. `None` for true background / already-promoted jobs.
    /// Main-thread only (event loop + action handlers) — never touched by the worker.
    pub tool_call_id: Option<String>,
    /// When true, the drain skips the completion nudge (Esc killed a still-blocking
    /// FG job; the abandoned turn must not auto-wake). Main-thread only.
    pub suppress_completion_nudge: bool,
    /// Mutable state shared with the worker thread (output / status / pid).
    pub shared: Arc<BashJobShared>,
}

impl BashJob {
    /// Snapshot the current lifecycle state (cloned out from under the lock).
    pub fn snapshot_status(&self) -> BashJobStatus {
        self.shared
            .status
            .lock()
            .map(|g| g.clone())
            .unwrap_or(BashJobStatus::Running)
    }

    /// Snapshot the captured output so far (cloned out from under the lock).
    pub fn output_snapshot(&self) -> String {
        self.shared
            .output
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// True while the job's status is still `Running`. Used by the `/bash` panel
    /// (a later stage) to badge live jobs.
    #[allow(dead_code)]
    pub fn is_running(&self) -> bool {
        matches!(self.snapshot_status(), BashJobStatus::Running)
    }

    /// True while this job still parks a main-turn tool call (foreground bash).
    pub fn is_blocking(&self) -> bool {
        self.tool_call_id.is_some() && self.is_running()
    }

    /// Clear the FG wall deadline so a promoted job no longer times out.
    pub fn clear_deadline(&self) {
        if let Ok(mut d) = self.shared.deadline.lock() {
            *d = None;
        }
    }

    /// Format a finished job's output the way synchronous `Bash::run` would via
    /// [`crate::tool::shell::finalize_output`] — used when FG completion delivers
    /// a tool result (not a `bash_output` poll).
    pub fn format_tool_result(
        &self,
        saving: bool,
        log_dir: Option<&std::path::Path>,
    ) -> String {
        use crate::tool::shell::{finalize_output, OutputOpts, ShellExit};

        let raw = self.output_snapshot();
        match self.snapshot_status() {
            BashJobStatus::Done(code) => {
                // Worker stores -1 when the OS reported no exit code (signal).
                let exit = if code < 0 {
                    ShellExit::Code(None)
                } else {
                    ShellExit::Code(Some(code))
                };
                finalize_output(
                    &self.command,
                    raw,
                    exit,
                    &OutputOpts {
                        saving,
                        log_dir: log_dir.map(|p| p.to_path_buf()),
                    },
                )
            }
            BashJobStatus::Killed => {
                if raw.is_empty() {
                    "job killed\nexit code: ?".to_string()
                } else {
                    finalize_output(
                        &self.command,
                        raw,
                        ShellExit::Code(None),
                        &OutputOpts {
                            saving,
                            log_dir: log_dir.map(|p| p.to_path_buf()),
                        },
                    )
                }
            }
            BashJobStatus::Error(msg) => {
                // Timeout / spawn failure — match capture_raw Early (plain message).
                if raw.is_empty() {
                    msg
                } else {
                    format!("{raw}\n{msg}")
                }
            }
            BashJobStatus::Running => {
                // Should not be called while still running; defensive.
                if raw.is_empty() {
                    "(still running)".to_string()
                } else {
                    raw
                }
            }
        }
    }

    /// Elapsed wall-clock seconds for the panel timer: frozen at the terminal
    /// instant once the job finished/was killed, else live since start.
    pub fn elapsed_secs(&self) -> u64 {
        let ended = self.shared.ended_at.lock().ok().and_then(|g| *g);
        match ended {
            Some(end) => end.saturating_duration_since(self.started_at).as_secs(),
            None => self.started_at.elapsed().as_secs(),
        }
    }

    /// Idempotently tee this job's full captured `raw` output to `log_dir`,
    /// via the SAME tee machinery `tool::shell::finalize_output` uses
    /// (`<log_dir>/<epoch_ms>_<slug>.log`, lazy `create_dir_all`, GC after
    /// write). The FIRST qualifying `bash_output` poll writes the file and
    /// remembers its path; every later poll reuses that same path WITHOUT
    /// rewriting it — the job's buffer keeps growing, but the model's
    /// full-output pointer for this job must stay stable. A write failure
    /// (bad dir, clock, IO) leaves `tee_path` `None` so a later poll can retry.
    pub fn ensure_tee_log(
        &self,
        log_dir: &std::path::Path,
        raw: &str,
    ) -> Option<std::path::PathBuf> {
        if let Ok(existing) = self.shared.tee_path.lock() {
            if let Some(path) = existing.as_ref() {
                return Some(path.clone());
            }
        }
        let path = crate::tool::shell::write_tee_log(log_dir, &self.command, raw)?;
        if let Ok(mut slot) = self.shared.tee_path.lock() {
            *slot = Some(path.clone());
        }
        Some(path)
    }
}

/// Decide + render a finished background job's `bash_output` text when the
/// model passed NEITHER `pattern` NOR `tail_lines` (the runtime only reaches
/// this once ALL of that plus a `Done` status and a non-empty buffer already
/// hold — see `app::runtime::stream::tools::approval`'s `bash_output` arm, the
/// only caller). Mirrors [`crate::tool::shell::finalize_output`]'s "saving"
/// filter + `[filter: ...]` marker exactly, so a finished background job gets
/// the SAME noise-trimming as synchronous `bash`/`git_operator` once it's
/// done — but is a pure function (no IO, no job/session state) so the
/// decision is unit-testable in isolation; the caller applies the tee
/// side-effect ([`BashJob::ensure_tee_log`]) using the returned `should_tee`
/// flag.
///
/// Returns `(text, should_tee)`: `text` is the model-visible body WITHOUT the
/// leading status line (the caller prepends that) and WITHOUT any
/// `full-output:` tee marker (that path is only known after the caller's tee
/// IO runs). `should_tee` is true when `saving` is on AND either the filter
/// actually changed something or the job exited non-zero — the same "might
/// have lost information" heuristic `finalize_output` uses for its
/// tee-write condition.
pub(crate) fn render_finished_output(
    command: &str,
    out: &str,
    exit_code: i32,
    saving: bool,
) -> (String, bool) {
    if !saving {
        return (out.to_string(), false);
    }

    let outcome = crate::tool::shell_filter::filter_output(command, out, Some(exit_code));
    let should_tee = outcome.changed || exit_code != 0;
    let text = if outcome.changed {
        if let Some(name) = outcome.filter_name {
            let raw_lines = out.lines().count();
            let out_lines = outcome.text.lines().count();
            format!(
                "{}\n[filter: {name}, {raw_lines} -> {out_lines} lines]",
                outcome.text.trim_end_matches('\n')
            )
        } else {
            outcome.text
        }
    } else {
        outcome.text
    };
    (text, should_tee)
}

/// Trim a job's captured output to a BOUNDED tail for a GUI stream tab: keep the LAST
/// ~200 lines, then cap to the last ~16000 chars (char-based, so multi-byte UTF-8 is
/// never sliced mid-codepoint). DELIBERATELY larger than the `/bash` panel's own
/// `tail_output` (~40 lines / ~4000 chars) — a stream tab is a scrollable dedicated
/// view, not a compact panel preview — but still bounded so a chatty long-running job's
/// per-client snapshot stays a sane size (the whole buffer is already ≤
/// [`crate::config::MAX_TOOL_OUTPUT_CHARS`] anyway). Used ONLY by the hub's per-client
/// stream-view projection (`stream_deltas`), never the shared snapshot path.
pub fn stream_output_tail(full: &str) -> String {
    const MAX_LINES: usize = 200;
    const MAX_CHARS: usize = 16_000;

    // Last ~MAX_LINES lines (preserving their order).
    let lines: Vec<&str> = full.lines().collect();
    let start = lines.len().saturating_sub(MAX_LINES);
    let mut tail = lines[start..].join("\n");

    // Then cap to the last MAX_CHARS chars so a single huge line can't blow the budget.
    let len = tail.chars().count();
    if len > MAX_CHARS {
        tail = tail.chars().skip(len - MAX_CHARS).collect();
    }
    tail
}

/// Append `chunk` to the shared output buffer, ANSI-stripping it first and then
/// capping the WHOLE buffer to the last [`crate::config::MAX_TOOL_OUTPUT_CHARS`]
/// chars (so the buffer mirrors the inline tool's last-N-chars cap and can never
/// grow unbounded for a long-lived job). Per-chunk stripping is the pragmatic v1
/// — an ANSI escape split across two reads only leaks cosmetically.
fn append_capped(shared: &BashJobShared, chunk: &str) {
    const MAX_CHARS: usize = crate::config::MAX_TOOL_OUTPUT_CHARS;
    let stripped = crate::dto::chat::strip_ansi(chunk);
    if let Ok(mut buf) = shared.output.lock() {
        buf.push_str(&stripped);
        // Keep only the last MAX_CHARS characters. `char`-based so multi-byte
        // UTF-8 is never sliced mid-codepoint.
        let len = buf.chars().count();
        if len > MAX_CHARS {
            let tail: String = buf.chars().skip(len - MAX_CHARS).collect();
            *buf = tail;
        }
    }
}

/// Spawn a bash job: run `command` via `sh -c` in `cwd`, streaming the merged
/// stdout+stderr into the returned job's shared buffer, and signal `done_tx` with
/// the job `id` when the child exits (or times out while still FG-blocking).
/// Returns the [`BashJob`] IMMEDIATELY — the worker thread owns the wait.
///
/// - `tool_call_id`: `Some(call.id)` for foreground bash (parks the turn);
///   `None` for true background.
/// - `timeout_ms`: FG wall deadline only; `None` / ignored for true BG. Cleared
///   on promote so a backgrounded job no longer times out.
///
/// Models the exec on [`crate::tool::shell::run_shell_capture`] but WITHOUT the
/// blocking wait: the child's pid is recorded into `shared.pid` as soon as it is
/// spawned (so `bash_kill` can reach it), reader threads stream stdout+stderr into
/// `shared.output` as they arrive, and the worker thread sets the terminal status
/// once the child exits — leaving a `Killed` status untouched if `bash_kill` won
/// the race. FG timeout: if `deadline` elapses while still Running, kill the child
/// and mark `Error("command timed out after …ms")`.
pub fn spawn_bash_job(
    id: usize,
    command: String,
    cwd: std::path::PathBuf,
    done_tx: Option<UnboundedSender<usize>>,
    tool_call_id: Option<String>,
    timeout_ms: Option<u64>,
) -> BashJob {
    let deadline = timeout_ms.map(|ms| Instant::now() + Duration::from_millis(ms));
    let shared = Arc::new(BashJobShared {
        output: Mutex::new(String::new()),
        status: Mutex::new(BashJobStatus::Running),
        pid: Mutex::new(None),
        ended_at: Mutex::new(None),
        tee_path: Mutex::new(None),
        deadline: Mutex::new(deadline),
    });
    let job = BashJob {
        id,
        command: command.clone(),
        started_at: Instant::now(),
        tool_call_id,
        suppress_completion_nudge: false,
        shared: Arc::clone(&shared),
    };

    // The worker thread owns the child + its wait. It must run with NO tokio
    // runtime in context (same constraint as the deferred lane), so it is a plain
    // std::thread.
    thread::spawn(move || {
        // Spawn the child, capturing stdout + stderr separately so each can be
        // streamed by its own reader thread.
        let mut child = match crate::tool::shell::os_shell_command(&command)
            .current_dir(&cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => {
                if let Ok(mut st) = shared.status.lock() {
                    *st = BashJobStatus::Error(format!("failed to spawn command: {e}"));
                }
                if let Ok(mut e) = shared.ended_at.lock() {
                    if e.is_none() {
                        *e = Some(Instant::now());
                    }
                }
                if let Some(tx) = &done_tx {
                    let _ = tx.send(id);
                }
                return;
            }
        };

        // Record the pid the instant the child exists so `bash_kill` can signal it
        // even before any output arrives.
        if let Ok(mut p) = shared.pid.lock() {
            *p = Some(child.id());
        }

        // Stream stdout + stderr concurrently into the shared buffer. Two reader
        // threads (not a select) keep it simple and avoid a deadlock where a full
        // stderr pipe blocks the child while we only drain stdout.
        let mut readers = Vec::new();
        if let Some(out) = child.stdout.take() {
            let sh = Arc::clone(&shared);
            readers.push(thread::spawn(move || stream_pipe(out, &sh)));
        }
        if let Some(err) = child.stderr.take() {
            let sh = Arc::clone(&shared);
            readers.push(thread::spawn(move || stream_pipe(err, &sh)));
        }

        // Poll wait with short sleeps so an FG deadline can fire without blocking
        // forever. True BG (deadline None) still wakes promptly on child exit.
        let mut timed_out_ms: Option<u64> = None;
        let wait_result = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => {
                    // Still running — check FG deadline (cleared on promote).
                    let hit = shared
                        .deadline
                        .lock()
                        .ok()
                        .and_then(|g| *g)
                        .filter(|d| Instant::now() >= *d);
                    if hit.is_some() {
                        // Kill the child so it does not orphan; mark timeout below.
                        let pid = shared.pid.lock().ok().and_then(|g| *g);
                        if let Some(pid) = pid {
                            kill_child(pid);
                        }
                        // Drain the wait so we don't zombie.
                        let _ = child.wait();
                        timed_out_ms = Some(timeout_ms.unwrap_or(0));
                        break Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "timeout",
                        ));
                    }
                    thread::sleep(Duration::from_millis(50));
                }
                Err(e) => break Err(e),
            }
        };
        for r in readers {
            let _ = r.join();
        }

        // Set the terminal status — but NEVER clobber a `Killed` set by
        // `bash_kill`, which raced the wait. Only a still-`Running` job advances to
        // Done/Error here.
        if let Ok(mut st) = shared.status.lock() {
            if matches!(*st, BashJobStatus::Running) {
                *st = if let Some(ms) = timed_out_ms {
                    BashJobStatus::Error(format!("command timed out after {ms}ms"))
                } else {
                    match wait_result {
                        // `.code()` is `None` when the process was terminated by a
                        // signal; report -1 so the status is still a concrete value.
                        Ok(status) => BashJobStatus::Done(status.code().unwrap_or(-1)),
                        Err(e) => BashJobStatus::Error(format!("wait failed: {e}")),
                    }
                };
                if let Ok(mut e) = shared.ended_at.lock() {
                    if e.is_none() {
                        *e = Some(Instant::now());
                    }
                }
            }
        }

        if let Some(tx) = &done_tx {
            let _ = tx.send(id);
        }
    });

    job
}

/// Read everything from `pipe` (a child's stdout or stderr) line-by-line and
/// append it into `shared`'s capped output buffer until EOF. Runs on its own
/// thread; returns when the pipe closes (at/after the child exits).
fn stream_pipe<R: std::io::Read>(pipe: R, shared: &BashJobShared) {
    let mut reader = BufReader::new(pipe);
    let mut line = String::new();
    loop {
        line.clear();
        // `read_line` keeps the trailing '\n', so the buffer reconstructs the
        // original stream layout. Lossy UTF-8 is fine — this is captured for
        // display, not byte-exact replay.
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => append_capped(shared, &line),
            Err(_) => break, // pipe error — stop reading this stream
        }
    }
}

/// Terminate a running background bash job: signal its child with `SIGTERM` (best
/// effort) and flip its status to `Killed`. A no-op on a job that already exited
/// (no pid, or already terminal) beyond setting `Killed`.
///
/// v1 is a single `SIGTERM` to the direct child pid — NOT a process-tree kill, so
/// a grandchild spawned by the shell may outlive the job. That is acceptable for
/// the common `long-running-command` case; tree-kill can be layered on later.
pub fn kill_bash_job(job: &BashJob) {
    // Flip to Killed FIRST so the worker's post-wait status set sees `Killed` and
    // leaves it (the `matches!(Running)` guard there).
    if let Ok(mut st) = job.shared.status.lock() {
        *st = BashJobStatus::Killed;
    }
    if let Ok(mut e) = job.shared.ended_at.lock() {
        if e.is_none() {
            *e = Some(Instant::now());
        }
    }
    // Signal the child if we have its pid. SIGTERM lets the process clean up; the
    // worker thread's `wait()` then unblocks and the reader pipes hit EOF.
    let pid = job.shared.pid.lock().ok().and_then(|g| *g);
    if let Some(pid) = pid {
        kill_child(pid);
    }
}

/// Best-effort terminate of a spawned child by pid.
///
/// SAFETY (unix): `kill(2)` with a pid we spawned and a standard signal number.
/// A failure (e.g. the child already reaped) is ignored — best effort.
#[cfg(unix)]
fn kill_child(pid: u32) {
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

/// Best-effort terminate of a spawned background-bash child by pid (Windows).
///
/// Uses `taskkill /T /F /PID` — a TREE kill — rather than a bare `TerminateProcess`.
/// Rationale: on Windows the job's direct child is `cmd.exe /C <command>` (see
/// [`crate::tool::shell::os_shell_command`]), so terminating just that pid would ORPHAN
/// the real work `cmd.exe` spawned and leave `bash_kill` ineffective; `/T` reaps the
/// whole descendant tree so the job actually stops, and `/F` forces it (console apps
/// routinely ignore the graceful WM_CLOSE). This is the ONE place a targeted whole-tree
/// stop is needed — the daemon's Job Object only tree-kills on daemon DEATH. It is
/// deliberately MORE thorough than the unix arm (a single `SIGTERM` to the direct child),
/// never less, and touches no unix behaviour.
///
/// Spawned with `CREATE_NO_WINDOW` (no console flash) and null stdio; fire-and-forget +
/// best-effort like the unix `libc::kill` — `taskkill` is near-instant, and a spawn
/// failure (already-exited pid, `taskkill` absent) is ignored.
#[cfg(windows)]
fn kill_child(pid: u32) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

    let pid_arg = pid.to_string();
    let _ = Command::new("taskkill")
        .args(["/T", "/F", "/PID", pid_arg.as_str()])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

#[cfg(test)]
#[path = "mod_test.rs"]
mod tests;
