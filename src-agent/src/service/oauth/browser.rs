//! Best-effort system-browser launch for the OAuth login flows.

use std::process::{Command, Stdio};

/// Open `url` in the system browser. Fire-and-forget: the child is spawned
/// and immediately detached (its stdio is discarded), so this never blocks on
/// the browser process. Returns `false` if no opener command could even be
/// launched (caller should fall back to printing the URL for the user to
/// open manually).
pub fn open_in_browser(url: &str) -> bool {
    spawn_opener(url).is_ok()
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
    Command::new("cmd")
        .args(["/c", "start", "", url])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn spawn_opener(_url: &str) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "no known browser opener for this platform",
    ))
}
