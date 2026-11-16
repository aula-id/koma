//! Shell tool: run arbitrary commands in the workspace.
//!
//! `bash` is RISKY — it requires user approval in Normal mode. Its safety
//! relies entirely on the approval gate, not path-sandboxing (unlike the
//! filesystem tools). Output is captured (stdout + stderr) and capped at the
//! last `MAX_READ_CHARS` characters (90k) so verbose build output doesn't
//! flood the context. The tool clipper then also applies the 2000-line cap.

use super::{Tool, ToolCtx};
use anyhow::Result;
use regex::Regex;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, OnceLock};

/// Exit status of a captured shell run, as seen by [`capture_raw`].
///
/// `Early` covers the three ways the child never produced a normal exit status
/// (spawn failure, wait failure, timeout): in all three the pre-refactor code
/// returned a bespoke message directly, with NO `exit code:` line and NO
/// truncation cap. [`finalize_output`] preserves that by short-circuiting on
/// `Early` and returning the raw message unchanged.
pub(crate) enum ShellExit {
    /// The process ran to completion. `Some(code)` is the numeric exit status;
    /// `None` means the OS reported none (rendered as `?`, matching the old code).
    Code(Option<i32>),
    /// Spawn/wait failed or the timeout fired; `raw` returned alongside this is
    /// already the complete, final message — do not format it further.
    Early,
}

/// Suppress the console window Windows would otherwise allocate for a freshly
/// spawned child process.
///
/// Causal chain: on a shortcut launch, `koma.exe`'s GUI calls `FreeConsole()` to
/// detach from whatever console it inherited (so the app window doesn't flash a
/// terminal behind it). A process with NO console of its own is a "console-less
/// parent" in Windows' eyes, and `CreateProcess` allocates a FRESH console for any
/// child spawned from one UNLESS the spawn opts out via `CREATE_NO_WINDOW` — so
/// without this, every background child (the extension host, an MCP stdio server, a
/// `git`/`ssh-keygen` subprocess, …) pops a visible black console window even though
/// nothing about the child needs one. `unix` has no such concept, so this is a no-op
/// there.
///
/// This is THE shared choke point for that flag: every `std::process::Command` spawn
/// that runs headlessly (never one that must show a window, and never a DETACHED
/// daemon spawn — those use `DETACHED_PROCESS` instead, a stronger flag that already
/// implies no console) should go through this rather than setting
/// `creation_flags(CREATE_NO_WINDOW)` by hand. See [`no_console_window_tokio`] for
/// the `tokio::process::Command` twin (its `creation_flags` is a same-signature
/// inherent method, not this trait, so it can't share this exact function).
#[cfg(windows)]
pub(crate) fn no_console_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(unix)]
pub(crate) fn no_console_window(_cmd: &mut Command) {}

/// [`no_console_window`]'s `tokio::process::Command` twin, for the async child
/// spawns (the extension host, stdio MCP servers, the security daemon) that build a
/// [`tokio::process::Command`] instead of a [`std::process::Command`]. Same
/// `CREATE_NO_WINDOW` flag, same causal chain — see [`no_console_window`]'s docs.
#[cfg(windows)]
pub(crate) fn no_console_window_tokio(cmd: &mut tokio::process::Command) {
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(unix)]
pub(crate) fn no_console_window_tokio(_cmd: &mut tokio::process::Command) {}

/// Build a [`Command`] that runs `command` through the platform's shell: unix
/// `sh -c <command>` (exactly as every caller ran it before this helper existed);
/// Windows prefers a Git-Bash-compatible `bash -c <command>` (same quoting/
/// globbing/`&&`/pipe semantics as unix `sh -c`) when Git for Windows is
/// discoverable (see [`find_git_bash`]), falling back to `cmd /C <command>`
/// only when no bash is found.
///
/// This is the ONE place every "run an arbitrary shell command string" call site
/// in the crate should go through, so the unix behavior never drifts between
/// them and the Windows Git-Bash-vs-cmd decision is made in exactly one spot.
#[cfg(unix)]
pub(crate) fn os_shell_command(command: &str) -> Command {
    let mut cmd = Command::new("sh");
    cmd.arg("-c").arg(command);
    cmd
}

#[cfg(windows)]
pub(crate) fn os_shell_command(command: &str) -> Command {
    let mut cmd = match find_git_bash() {
        // Plain `-c`, NOT a login shell (`-lc`) — matches unix `sh -c` semantics
        // exactly (no profile/rc sourcing), so behavior stays as close to unix as
        // this platform allows.
        Some(bash) => {
            let mut c = Command::new(&bash);
            c.arg("-c").arg(command);
            for (key, value) in git_bash_env(&bash) {
                c.env(key, value);
            }
            c
        }
        None => {
            let mut c = Command::new("cmd");
            c.arg("/C").arg(command);
            c
        }
    };
    // No console flash when the daemon/gui spawns a shell headlessly.
    no_console_window(&mut cmd);
    cmd
}

/// Last [`find_git_bash`] outcome. A found bash is kept (and re-checked on disk
/// each call); a miss is only trusted for [`GIT_BASH_RETRY`] so installing Git
/// while koma runs does not need a restart.
#[cfg(windows)]
static GIT_BASH: std::sync::Mutex<Option<(std::time::Instant, Option<PathBuf>)>> =
    std::sync::Mutex::new(None);

#[cfg(windows)]
static GIT_BASH_DIAGNOSED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

#[cfg(windows)]
const GIT_BASH_RETRY: std::time::Duration = std::time::Duration::from_secs(30);

/// Locate a Git-for-Windows `bash.exe`, trying (in order):
///
/// 1. `git --exec-path` — spawns `git.exe` off the host `%PATH%`; its stdout looks like
///    `<git-root>/mingw64/libexec/git-core`, so we drop three components to
///    `<git-root>` and check `<git-root>\bin\bash.exe` then `<git-root>\usr\bin\bash.exe`.
/// 2. The registry key `SOFTWARE\GitForWindows` value `InstallPath` under HKLM,
///    HKCU and HKLM\WOW6432Node, read via `reg query` (not a registry crate).
/// 3. Known install locations: `%ProgramFiles%`, `%ProgramFiles(x86)%`,
///    `%ProgramW6432%` `\Git` and `%LOCALAPPDATA%\Programs\Git`.
/// 4. The MSI fallback next to this executable: `<exe_dir>\shell\usr\bin\bash.exe`
///    (then `<exe_dir>\shell\bin\bash.exe`). System Git wins when it is installed.
///
/// Deliberately NEVER resolves a bare `bash` off `%PATH%` — on stock Windows
/// `C:\Windows\System32\bash.exe` is the WSL launcher, not a real shell.
///
/// The result is normalised with `winpath::win_normalize` so every consumer
/// sees one spelling. When nothing is found, the outcome of each probe is
/// written once to `error.log`.
#[cfg(windows)]
pub(crate) fn find_git_bash() -> Option<PathBuf> {
    let mut cache = GIT_BASH.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, found)) = cache.as_ref() {
        match found {
            Some(p) if p.is_file() => return Some(p.clone()),
            None if at.elapsed() < GIT_BASH_RETRY => return None,
            _ => {}
        }
    }
    let mut trace = Vec::new();
    let found = find_git_bash_via_exec_path(&mut trace)
        .or_else(|| find_git_bash_via_registry(&mut trace))
        .or_else(|| find_git_bash_via_known_paths(&mut trace))
        .or_else(|| find_git_bash_via_install_dir(&mut trace))
        .map(|p| PathBuf::from(super::winpath::win_normalize(&p.to_string_lossy())));
    if found.is_none() && !GIT_BASH_DIAGNOSED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        crate::model::store::append_global_error_log(
            "shell",
            &format!("Git Bash not found; probes: {}", trace.join("; ")),
        );
    }
    *cache = Some((std::time::Instant::now(), found.clone()));
    found
}

/// Check the two locations a Git-for-Windows root can hold `bash.exe`:
/// `<root>\bin\bash.exe` (a shim) and `<root>\usr\bin\bash.exe` (the real MSYS2
/// bash). Returns the first that exists on disk and records each try.
#[cfg(windows)]
fn bash_candidates(git_root: &str, trace: &mut Vec<String>) -> Option<PathBuf> {
    for candidate in super::winpath::git_bash_candidates(git_root) {
        let found = Path::new(&candidate).is_file();
        trace.push(format!(
            "{candidate}: {}",
            if found { "found" } else { "missing" }
        ));
        if found {
            return Some(PathBuf::from(candidate));
        }
    }
    None
}

/// Strategy 1: ask `git` itself where it lives via `git --exec-path`.
#[cfg(windows)]
fn find_git_bash_via_exec_path(trace: &mut Vec<String>) -> Option<PathBuf> {
    let mut cmd = Command::new("git");
    cmd.arg("--exec-path")
        .env("PATH", crate::coding::environment::host_path());
    no_console_window(&mut cmd);
    let output = match cmd.output() {
        Ok(o) => o,
        Err(e) => {
            trace.push(format!("git --exec-path: {e}"));
            return None;
        }
    };
    let exec_path = String::from_utf8_lossy(&output.stdout);
    trace.push(format!(
        "git --exec-path: status {} output '{}'",
        output.status,
        exec_path.trim()
    ));
    if !output.status.success() {
        return None;
    }
    let root = super::winpath::git_root_from_exec_path(&exec_path)?;
    bash_candidates(&root, trace)
}

/// Strategy 2: the install marker Git for Windows writes to the registry, read
/// with `reg query` from every hive/view a per-user or 32-bit install may use.
#[cfg(windows)]
fn find_git_bash_via_registry(trace: &mut Vec<String>) -> Option<PathBuf> {
    for key in [
        r"HKLM\SOFTWARE\GitForWindows",
        r"HKCU\SOFTWARE\GitForWindows",
        r"HKLM\SOFTWARE\WOW6432Node\GitForWindows",
    ] {
        let mut cmd = Command::new("reg");
        cmd.args(["query", key, "/v", "InstallPath"]);
        no_console_window(&mut cmd);
        let output = match cmd.output() {
            Ok(o) if o.status.success() => o,
            Ok(o) => {
                trace.push(format!("reg {key}: status {}", o.status));
                continue;
            }
            Err(e) => {
                trace.push(format!("reg {key}: {e}"));
                continue;
            }
        };
        let text = String::from_utf8_lossy(&output.stdout);
        match super::winpath::parse_reg_sz(&text) {
            Some(root) => {
                trace.push(format!("reg {key}: InstallPath {root}"));
                if let Some(bash) = bash_candidates(&root, trace) {
                    return Some(bash);
                }
            }
            None => trace.push(format!("reg {key}: no InstallPath value")),
        }
    }
    None
}

/// Strategy 3: fixed locations Git for Windows commonly installs to, checked
/// as a plain existence probe (no subprocess involved).
#[cfg(windows)]
fn find_git_bash_via_known_paths(trace: &mut Vec<String>) -> Option<PathBuf> {
    use super::winpath::{win_join, win_key};
    let mut roots = vec![
        r"C:\Program Files\Git".to_string(),
        r"C:\Program Files (x86)\Git".to_string(),
    ];
    for var in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Ok(base) = std::env::var(var) {
            roots.push(win_join(&base, "Git"));
        }
    }
    if let Ok(base) = std::env::var("LOCALAPPDATA") {
        roots.push(win_join(&base, r"Programs\Git"));
    }
    let mut seen = std::collections::HashSet::new();
    roots.retain(|r| seen.insert(win_key(r)));
    roots.iter().find_map(|root| bash_candidates(root, trace))
}

/// Strategy 4: shell runtime shipped beside `koma.exe` (`INSTALLDIR\shell`).
#[cfg(windows)]
fn find_git_bash_via_install_dir(trace: &mut Vec<String>) -> Option<PathBuf> {
    let exe = match std::env::current_exe() {
        Ok(e) => e,
        Err(e) => {
            trace.push(format!("current_exe: {e}"));
            return None;
        }
    };
    let parent = exe.parent()?;
    let root = super::winpath::win_join(&parent.to_string_lossy(), "shell");
    bash_candidates(&root, trace)
}

/// `usr\bin` that holds `grep` / `ls` / `sed` for this `bash.exe`.
///
/// `<root>\usr\bin\bash.exe` returns that directory. `<root>\bin\bash.exe` (the
/// Git for Windows shim) returns `<root>\usr\bin`.
#[cfg(test)]
pub(crate) fn git_bash_usr_bin(bash: &Path) -> PathBuf {
    if let Some(parent) = bash.parent() {
        if file_name_eq(parent, "bin") {
            if let Some(grand) = parent.parent() {
                if file_name_eq(grand, "usr") {
                    return parent.to_path_buf();
                }
                return grand.join("usr").join("bin");
            }
        }
        return parent.to_path_buf();
    }
    PathBuf::from("usr").join("bin")
}

#[cfg(test)]
fn file_name_eq(path: &Path, expect: &str) -> bool {
    path.file_name()
        .map(|name| name.to_string_lossy().eq_ignore_ascii_case(expect))
        .unwrap_or(false)
}

/// Env for a Git Bash child: MSYS tools on `PATH`, without a login shell.
#[cfg(windows)]
pub(crate) fn git_bash_env(bash: &Path) -> Vec<(String, String)> {
    let mut vars = Vec::with_capacity(2);
    let usr_bin = PathBuf::from(super::winpath::git_bash_usr_bin_str(
        &bash.to_string_lossy(),
    ));
    if usr_bin.is_dir() {
        let old = crate::coding::environment::host_path();
        if let Ok(joined) =
            std::env::join_paths(std::iter::once(usr_bin).chain(std::env::split_paths(&old)))
        {
            vars.push(("PATH".into(), joined.to_string_lossy().into_owned()));
        }
    }
    vars.push(("MSYSTEM".into(), "MINGW64".into()));
    vars
}

/// Spawn `command` via `sh -c` in `cwd`, capture stdout+stderr, strip ANSI, and
/// return the combined output alongside its exit status. Bounded by `timeout_ms`
/// (the child keeps running on a drain thread past the timeout, but the caller is
/// freed with a timeout message so the UI/turn never stalls). Capturing-only (no
/// TTY); never panics.
pub(crate) fn capture_raw(command: &str, cwd: &Path, timeout_ms: u64) -> (String, ShellExit) {
    // Spawn the child, capturing stdout + stderr.
    let child = match os_shell_command(command)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(e) => {
            return (
                format!("error: failed to spawn command: {e}"),
                ShellExit::Early,
            )
        }
    };

    // Wait with timeout using a helper thread + channel.
    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    // `child` is moved into the thread; we get the result back via the channel.
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    let timeout = std::time::Duration::from_millis(timeout_ms);
    let output = match rx.recv_timeout(timeout) {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return (format!("error: command failed: {e}"), ShellExit::Early),
        Err(_) => {
            // The child thread owns the child now — we can't kill it here, but we
            // still return a timeout message so the caller doesn't stall. The thread
            // drains on its own when the child finishes.
            return (
                format!("command timed out after {timeout_ms}ms"),
                ShellExit::Early,
            );
        }
    };

    // Combine stdout + stderr into one string.
    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    // Strip ANSI color codes so git/cargo colorized output doesn't bleed into
    // tool results, history, the transcript, and the rolling summary.
    let combined = crate::dto::chat::strip_ansi(&combined);

    (combined, ShellExit::Code(output.status.code()))
}

/// Options controlling [`finalize_output`]'s post-processing of captured output.
pub struct OutputOpts {
    /// When true, run the output through [`crate::tool::shell_filter::filter_output`]
    /// before the truncation cap, to trim known-noisy command output.
    pub saving: bool,
    /// When `saving` is true and this is `Some`, the directory the full
    /// (untruncated, ANSI-stripped) output gets tee'd to for commands worth
    /// keeping around — see [`finalize_output`]'s write condition.
    pub log_dir: Option<PathBuf>,
}

/// Turn a [`capture_raw`] result into the final tool-output string: optionally
/// filters noisy output (when `opts.saving`), tees the full output to disk when
/// warranted, applies the shared truncation cap, and appends the `exit code: N`
/// line. `Early` exits bypass all of this and are returned unchanged, matching
/// the pre-refactor early-return behavior exactly (no tee — those are bespoke
/// error strings, not real command output).
pub(crate) fn finalize_output(
    command: &str,
    raw: String,
    exit: ShellExit,
    opts: &OutputOpts,
) -> String {
    let exit_code_num = match exit {
        ShellExit::Early => return raw,
        ShellExit::Code(code) => code,
    };
    let exit_code_str = exit_code_num
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".to_string());

    if !opts.saving {
        return format_captured_output(raw, &exit_code_str);
    }

    const MAX_CHARS: usize = crate::config::MAX_READ_CHARS;
    let will_truncate = raw.chars().count() > MAX_CHARS;
    // `None` (OS reported no code — rendered as `?`) is treated as non-clean too,
    // since it's ambiguous whether the command actually succeeded.
    let exit_nonzero = exit_code_num.map(|c| c != 0).unwrap_or(true);

    let outcome = crate::tool::shell_filter::filter_output(command, &raw, exit_code_num);
    let changed = outcome.changed;
    let text = if changed {
        if let Some(name) = outcome.filter_name {
            let raw_lines = raw.lines().count();
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

    // Tee the full raw output to disk when the filter actually trimmed
    // something, the cap below would truncate it, or the command didn't exit
    // clean — i.e. whenever there's a real chance the model-visible text lost
    // information the full log would still have.
    let tee_path = match opts.log_dir.as_ref() {
        Some(dir) if changed || will_truncate || exit_nonzero => write_tee_log(dir, command, &raw),
        _ => None,
    };

    format_captured_output_tee(text, &exit_code_str, tee_path.as_deref())
}

/// Build a filesystem-safe slug from the first two whitespace-separated words of
/// `command`: lowercased, every char outside `[a-z0-9]` replaced with `-`,
/// consecutive dashes collapsed, capped at 40 chars. Used to name tee log files
/// so they're recognizable at a glance (`1720000000000_cargo-build.log`).
fn command_slug(command: &str) -> String {
    let words: Vec<&str> = command.split_whitespace().take(2).collect();
    let joined = words.join(" ").to_lowercase();

    let mut slug = String::with_capacity(joined.len());
    let mut last_dash = false;
    for c in joined.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    slug.chars().take(40).collect()
}

/// Write the full (untruncated) `raw` output to a new log file under `log_dir`,
/// named `<epoch_ms>_<slug>.log`, then GC the directory. Returns the file's
/// absolute path on success. Every failure mode (can't create the dir, can't get
/// the clock, can't write the file) is a silent no-op — a tee failure must never
/// break the tool result.
///
/// `pub(crate)` so [`crate::app::bgbash`] can reuse the exact same tee
/// machinery for finished background jobs (`bash_output` polls), instead of
/// duplicating the file-naming/GC logic.
pub(crate) fn write_tee_log(log_dir: &Path, command: &str, raw: &str) -> Option<PathBuf> {
    if std::fs::create_dir_all(log_dir).is_err() {
        return None;
    }
    let epoch_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    let filename = format!("{epoch_ms}_{}.log", command_slug(command));
    let path = log_dir.join(&filename);
    if std::fs::write(&path, raw).is_err() {
        return None;
    }

    gc_log_dir(log_dir);

    Some(path.canonicalize().unwrap_or(path))
}

/// GC `log_dir`'s tee logs: delete the oldest `.log` files until at most 50
/// remain AND the total size is at most 100 MiB. Filenames are epoch-ms
/// prefixed, so a plain filename sort is chronological (oldest first). Silent
/// on any IO error (a GC failure must never break the tool result).
fn gc_log_dir(log_dir: &Path) {
    const MAX_COUNT: usize = 50;
    const MAX_BYTES: u64 = 100 * 1024 * 1024;

    let entries = match std::fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut logs: Vec<(String, u64)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        logs.push((name.to_string(), size));
    }
    logs.sort_by(|a, b| a.0.cmp(&b.0));

    let mut total: u64 = logs.iter().map(|(_, s)| s).sum();
    let mut count = logs.len();
    for (name, size) in logs.iter() {
        if count <= MAX_COUNT && total <= MAX_BYTES {
            break;
        }
        if std::fs::remove_file(log_dir.join(name)).is_ok() {
            total = total.saturating_sub(*size);
            count -= 1;
        }
    }
}

/// Run `command` via `sh -c` in `cwd`, capturing stdout+stderr, and return the
/// combined output: ANSI-stripped, capped to the LAST [`crate::config::MAX_READ_CHARS`]
/// chars, with a trailing `exit code: N` line. Bounded by `timeout_ms` (the child
/// keeps running on a drain thread past the timeout, but the caller is freed with a
/// timeout message so the UI/turn never stalls).
///
/// This is THE shared shell-execution primitive: the model-callable `bash` tool
/// ([`Bash::run`]) and the `!` user-shell shortcut (`app::runtime::actions::chat`)
/// both funnel through here so the output cap, ANSI stripping, and timeout bound can
/// never diverge between them. Capturing-only (no TTY); never panics. Thin wrapper
/// over [`capture_raw`] + [`finalize_output`] with filtering off, preserving the
/// exact original behavior for every existing caller.
pub fn run_shell_capture(command: &str, cwd: &Path, timeout_ms: u64) -> String {
    let (raw, exit) = capture_raw(command, cwd, timeout_ms);
    finalize_output(
        command,
        raw,
        exit,
        &OutputOpts {
            saving: false,
            log_dir: None,
        },
    )
}

/// Format captured command output: ANSI must already be stripped. Applies the
/// shared output cap (last [`crate::config::MAX_READ_CHARS`] chars), adds a
/// truncation notice when trimmed, ensures a trailing newline, and appends
/// `exit code: <code>`. Shared by [`run_shell_capture`] and `git_operator`. Thin
/// wrapper over [`format_captured_output_tee`] with no tee path — wording is
/// byte-identical to before tee support existed.
pub(crate) fn format_captured_output(text: String, exit_code: &str) -> String {
    format_captured_output_tee(text, exit_code, None)
}

/// Like [`format_captured_output`], but when `tee_path` is `Some`, appends a
/// `full-output: <path>` line after the content (and after the `[filter: ...]`
/// marker, when present — that marker is already baked into `text`) and before
/// the `exit code: N` line; the truncation notice (when the output is actually
/// truncated) also points at that path instead of the generic "redirect to a
/// file" wording. With `tee_path: None` the output is byte-identical to the
/// pre-tee behavior.
pub(crate) fn format_captured_output_tee(
    text: String,
    exit_code: &str,
    tee_path: Option<&Path>,
) -> String {
    const MAX_CHARS: usize = crate::config::MAX_READ_CHARS;
    let truncated;
    let tail: String = if text.chars().count() > MAX_CHARS {
        truncated = true;
        text.chars()
            .rev()
            .take(MAX_CHARS)
            .collect::<String>()
            .chars()
            .rev()
            .collect()
    } else {
        truncated = false;
        text
    };

    let mut out = String::new();
    if truncated {
        match tee_path {
            Some(p) => out.push_str(&format!(
                "... (output truncated to last {MAX_CHARS} chars; full output written to {} — read it if you need the full output)\n",
                p.display()
            )),
            None => out.push_str(&format!("... (output truncated to last {MAX_CHARS} chars; redirect to a file and read it if you need the full output)\n")),
        }
    }
    out.push_str(&tail);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    if let Some(p) = tee_path {
        out.push_str(&format!("full-output: {}\n", p.display()));
    }
    out.push_str(&format!("exit code: {exit_code}"));
    out
}

/// Run a shell command in the workspace directory.
pub struct Bash;
impl Tool for Bash {
    fn name(&self) -> &'static str {
        "bash"
    }
    fn description(&self) -> &'static str {
        "Run a shell command for builds, tests, and shell-specific behavior. Use dir_list for directory contents and glob/grep for file discovery. \
         A command that itself runs git is rejected. \
         Output is capped at 90k chars / 2000 lines — narrow the command or pipe through head/grep; do not dump huge logs. \
         Known noisy commands (cargo, npm, pip, docker, make) may be auto-compressed; a [filter: <name>, N -> M lines] marker shows when. \
         If output was compressed, truncated, or the command failed, the complete output is saved and a 'full-output: <path>' line points at it — read that file with offset/limit; do not dump it whole or re-run the command."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to run (passed to sh -c)."
                },
                "timeout_ms": {
                    "type": "number",
                    "description": "Timeout in milliseconds (default 120000). Ignored when run_in_background is true."
                },
                "run_in_background": {
                    "type": "boolean",
                    "description": "If true, run detached and return a job id immediately (e.g. bash-1). Use for long-running / open-ended commands so the turn is not blocked. Poll with bash_output, stop with bash_kill. Defaults to false."
                }
            },
            "required": ["command"]
        })
    }
    fn run(&self, ctx: &ToolCtx, args: &Value) -> Result<String> {
        let command = args
            .get("command")
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("missing required string argument 'command'"))?;

        // Fail-closed: no writable workspace roots (e.g. SDLC execute/integrate
        // with missing/invalid binding) → refuse rather than running against an
        // empty/primary cwd.
        if ctx.workspaces.is_empty() || ctx.workspace.as_os_str().is_empty() {
            return Ok(
                "error: no writable workspace root — SDLC execute/integrate binding \
                 missing or invalid; cannot run bash against primary"
                    .to_string(),
            );
        }

        // Redirect git commands to git_operator, which handles SSH key injection
        // and destructive-operation guards. Bash can't do either safely.
        // Match `git` as a command word anywhere in the command (not just at the
        // start), so cheats like `cd /tmp && git status` or `echo $(git log)`
        // are caught too.
        static GIT_RE: OnceLock<Regex> = OnceLock::new();
        let git_re = GIT_RE.get_or_init(|| crate::re_util::static_re(r"(?:^|[\s;&|(])git\b"));
        if git_re.is_match(command.trim()) {
            return Ok(
                "error: use the git_operator tool for git commands, not bash. \
                 git_operator runs git directly (no shell-injection risk), injects the \
                 session SSH key automatically, and gates destructive operations. \
                 Example: git_operator({\"args\": [\"log\", \"--oneline\", \"-5\"]})"
                    .to_string(),
            );
        }

        let timeout_ms: u64 = args
            .get("timeout_ms")
            .and_then(Value::as_u64)
            .unwrap_or(120_000);

        // Same execution primitive as the `!` user-shell shortcut
        // (`run_shell_capture`), but with the session's saving/tee settings
        // applied — `run_shell_capture` itself always runs with saving off.
        let run = || {
            let (raw, exit) = capture_raw(command, &ctx.workspace, timeout_ms);
            let failed = !matches!(exit, ShellExit::Code(Some(0)));
            let opts = OutputOpts {
                saving: ctx.bash_saving,
                log_dir: ctx.bash_log_dir.clone(),
            };
            (finalize_output(command, raw, exit, &opts), failed)
        };
        if super::recovery::listing_command(args) {
            super::recovery::shell_listing(ctx, args, run)
        } else {
            Ok(run().0)
        }
    }
}

/// Poll a background bash job's status + captured output.
///
/// Like [`Task`](super::task::Task), this tool is advertised to the model but
/// NEVER dispatched through [`Tool::run`]: the runtime
/// (`app::runtime::stream::process_tools`) intercepts a `bash_output` call BEFORE
/// the generic dispatch path, looks up the job in the session's `bash_jobs`
/// registry, and answers synchronously. The `run` impl exists only to satisfy the
/// [`Tool`] trait and must never be reached.
pub struct BashOutput;
impl Tool for BashOutput {
    fn name(&self) -> &'static str {
        "bash_output"
    }
    fn description(&self) -> &'static str {
        "Return the current status and captured output of a background bash job \
         (one started by bash with run_in_background=true). Poll this to watch a \
         long-running command's progress. Optionally filter the output with \
         `pattern` (a regex, grep-style) and/or limit to the last `tail_lines` \
         lines to save tokens on noisy jobs. Once a job finishes, its output may be \
         auto-compressed the same way as foreground bash ([filter: ...] marker + 'full-output: <path>' file); \
         pass tail_lines or pattern to get the raw buffer instead."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job id returned when the background command was started (e.g. \"bash-1\")."
                },
                "tail_lines": {
                    "type": "number",
                    "description": "If set (>0), return only the last N lines of output (applied AFTER any pattern filter). Omit for the full captured tail."
                },
                "pattern": {
                    "type": "string",
                    "description": "If set, a regular expression; only output lines matching it are returned (like grep). Combine with tail_lines to get the last N matching lines."
                }
            },
            "required": ["job_id"]
        })
    }
    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: bash_output must be handled by the runtime".into())
    }
}

/// Terminate a running background bash job.
///
/// Like [`Task`](super::task::Task), this tool is advertised to the model but
/// NEVER dispatched through [`Tool::run`]: the runtime intercepts a `bash_kill`
/// call BEFORE the generic dispatch path, kills the job (SIGTERM to its child),
/// and answers synchronously. The `run` impl exists only to satisfy the [`Tool`]
/// trait and must never be reached.
pub struct BashKill;
impl Tool for BashKill {
    fn name(&self) -> &'static str {
        "bash_kill"
    }
    fn description(&self) -> &'static str {
        "Terminate a running background bash job (one started by bash with \
         run_in_background=true)."
    }
    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "job_id": {
                    "type": "string",
                    "description": "The job id of the background command to stop (e.g. \"bash-1\")."
                }
            },
            "required": ["job_id"]
        })
    }
    fn run(&self, _ctx: &ToolCtx, _args: &Value) -> Result<String> {
        // Intercepted by the runtime before dispatch; never actually called.
        Ok("error: bash_kill must be handled by the runtime".into())
    }
}

#[cfg(test)]
#[path = "shell_test.rs"]
mod tests;
