//! Background-bash registry: run a shell command DETACHED and poll it later.
//!
//! The model-callable `bash` tool normally runs INLINE (or on the deferred
//! `std::thread` lane) and the turn waits for the command to finish. With
//! `run_in_background: true` the runtime instead registers a [`BashJob`] here:
//! the command is spawned on its own worker thread, the tool returns a job id
//! IMMEDIATELY, and the model polls the captured output with `bash_output` /
//! stops it with `bash_kill` (both intercepted in
//! `app::runtime::stream::process_tools`, mirroring the `task` tool).
//!
//! Concurrency shape mirrors the rest of the crate's off-thread work: a plain
//! `std::thread` owns the blocking child wait (NOT a tokio task — the shell
//! child must run with no tokio runtime in context, same as the deferred lane),
//! and the job's mutable state lives behind an `Arc<`[`BashJobShared`]`>` shared
//! between that worker and the registry entry. Completion is signalled over an
//! `UnboundedSender<usize>` (the job id) so the event loop can surface a toast.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

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
}

/// One registered background bash job: its identity, the command, when it
/// started, and the shared mutable state the worker thread updates.
pub struct BashJob {
    /// Stable per-session id, allocated from `SessionRuntime::next_bash_job_id`.
    /// Surfaced to the model as `bash-<id>`.
    pub id: usize,
    /// The shell command this job runs. Read by the `/bash` panel + chat-line
    /// rendering (a later stage); kept now so the registry entry is complete.
    #[allow(dead_code)]
    pub command: String,
    /// Wall-clock instant the job was registered. Read by the `/bash` panel (a
    /// later stage) to show how long a job has been running.
    #[allow(dead_code)]
    pub started_at: Instant,
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

/// Spawn a background bash job: run `command` via `sh -c` in `cwd`, streaming the
/// merged stdout+stderr into the returned job's shared buffer, and signal `done_tx`
/// with the job `id` when the child exits. Returns the [`BashJob`] IMMEDIATELY —
/// the worker thread owns the blocking wait, so the caller never stalls.
///
/// Models the exec on [`crate::tool::shell::run_shell_capture`] but WITHOUT the
/// blocking wait: the child's pid is recorded into `shared.pid` as soon as it is
/// spawned (so `bash_kill` can reach it), reader threads stream stdout+stderr into
/// `shared.output` as they arrive, and the worker thread sets the terminal status
/// once the child exits — leaving a `Killed` status untouched if `bash_kill` won
/// the race.
pub fn spawn_bash_job(
    id: usize,
    command: String,
    cwd: std::path::PathBuf,
    done_tx: Option<UnboundedSender<usize>>,
) -> BashJob {
    let shared = Arc::new(BashJobShared {
        output: Mutex::new(String::new()),
        status: Mutex::new(BashJobStatus::Running),
        pid: Mutex::new(None),
    });
    let job = BashJob {
        id,
        command: command.clone(),
        started_at: Instant::now(),
        shared: Arc::clone(&shared),
    };

    // The worker thread owns the child + its wait. It must run with NO tokio
    // runtime in context (same constraint as the deferred lane), so it is a plain
    // std::thread.
    thread::spawn(move || {
        // Spawn the child, capturing stdout + stderr separately so each can be
        // streamed by its own reader thread.
        let mut child = match Command::new("sh")
            .arg("-c")
            .arg(&command)
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

        // Block until the child exits. The reader threads finish when their pipes
        // hit EOF (at/after child exit); join them so all output is captured before
        // we set the terminal status.
        let wait_result = child.wait();
        for r in readers {
            let _ = r.join();
        }

        // Set the terminal status — but NEVER clobber a `Killed` set by
        // `bash_kill`, which raced the wait. Only a still-`Running` job advances to
        // Done/Error here.
        if let Ok(mut st) = shared.status.lock() {
            if matches!(*st, BashJobStatus::Running) {
                *st = match wait_result {
                    // `.code()` is `None` when the process was terminated by a
                    // signal; report -1 so the status is still a concrete value.
                    Ok(status) => BashJobStatus::Done(status.code().unwrap_or(-1)),
                    Err(e) => BashJobStatus::Error(format!("wait failed: {e}")),
                };
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
    // Signal the child if we have its pid. SIGTERM lets the process clean up; the
    // worker thread's `wait()` then unblocks and the reader pipes hit EOF.
    let pid = job.shared.pid.lock().ok().and_then(|g| *g);
    if let Some(pid) = pid {
        // SAFETY: `kill(2)` with a pid we spawned and a standard signal number.
        // A failure (e.g. the child already reaped) is ignored — best effort.
        unsafe {
            libc::kill(pid as libc::pid_t, libc::SIGTERM);
        }
    }
}
