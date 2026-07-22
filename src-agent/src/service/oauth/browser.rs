//! Best-effort system-browser launch for the OAuth login flows.

use std::io::Write;
use std::process::{Command, Stdio};

/// Open `url` in the system browser. Fire-and-forget: the child is spawned
/// and immediately detached (its stdio is discarded), so this never blocks on
/// the browser process. Returns `false` if no opener command could even be
/// launched (caller should fall back to printing the URL for the user to
/// open manually).
pub fn open_in_browser(url: &str) -> bool {
    spawn_opener(url).is_ok()
}

/// Best-effort: write `text` to the system clipboard. Returns `false` if no
/// clipboard tool worked. Same same-machine assumption as `open_in_browser`.
pub fn copy_to_clipboard(text: &str) -> bool {
    spawn_copier(text)
}

/// Spawn `cmd` with a piped stdin, write `text`'s bytes into it, close the
/// pipe, and wait for the child to exit. Returns `false` if the command isn't
/// installed, the write fails, or the child exits non-zero. These clipboard
/// tools exit immediately (or fork-and-detach themselves, in `wl-copy`'s
/// case), so a plain `.wait()` is safe here — no timeout machinery needed
/// (contrast `controller::input::clipboard`'s read side, which guards a
/// potentially slow read with `recv_timeout`).
fn write_via(cmd: &mut Command, text: &str) -> bool {
    // No console flash on Windows (the `clip` arm below) — see
    // `tool::shell::no_console_window`'s docs for the `FreeConsole()` causal
    // chain this guards against. A no-op on the linux/macos arms.
    crate::tool::shell::no_console_window(cmd);
    let mut child = match cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    let write_ok = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(text.as_bytes()).is_ok())
        .unwrap_or(false);
    let wait_ok = child.wait().map(|s| s.success()).unwrap_or(false);
    write_ok && wait_ok
}

/// Linux: try `wl-copy` (Wayland) first, then `xclip -selection clipboard`
/// (X11) — the write-direction counterpart of `clipboard::try_wl_paste` /
/// `try_xclip`.
#[cfg(target_os = "linux")]
fn spawn_copier(text: &str) -> bool {
    if write_via(&mut Command::new("wl-copy"), text) {
        return true;
    }
    write_via(
        Command::new("xclip").args(["-selection", "clipboard"]),
        text,
    )
}

#[cfg(target_os = "macos")]
fn spawn_copier(text: &str) -> bool {
    write_via(&mut Command::new("pbcopy"), text)
}

#[cfg(target_os = "windows")]
fn spawn_copier(text: &str) -> bool {
    write_via(&mut Command::new("clip"), text)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn spawn_copier(_text: &str) -> bool {
    false
}

#[cfg(target_os = "linux")]
fn spawn_opener(url: &str) -> std::io::Result<()> {
    Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn spawn_opener(url: &str) -> std::io::Result<()> {
    Command::new("open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "windows")]
fn spawn_opener(url: &str) -> std::io::Result<()> {
    // cmd /c start splits unquoted URLs at '&'; rundll32 passes argv without shell parsing
    let mut cmd = Command::new("rundll32");
    cmd.args(["url.dll,FileProtocolHandler", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // rundll32 is a GUI-subsystem binary (no console of its own either way), but
    // set this anyway for uniformity with every other Windows spawn in the crate —
    // see `tool::shell::no_console_window`'s docs for the `FreeConsole()` chain.
    crate::tool::shell::no_console_window(&mut cmd);
    cmd.spawn().map(|_| ())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn spawn_opener(_url: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no known browser opener for this platform",
    ))
}
