//! Clipboard-image paste (Ctrl+V in Chat mode).
//!
//! Reads raw PNG bytes from the OS clipboard on a plain `std::thread`, using the
//! same thread + channel + `recv_timeout` pattern as `tool::shell::Bash`. The
//! bytes are sent back over a `std::sync::mpsc` channel stored in
//! `AppStateRest::clipboard_rx`; the event-loop drain picks them up next tick
//! and calls `AppStateRest::try_attach_image_bytes`.
//!
//! The backend is platform-specific and selected at compile time:
//!
//! * **Linux** — shells out to `wl-paste --type image/png` (Wayland) or
//!   `xclip -selection clipboard -t image/png -o` (X11).
//! * **macOS** — tries `pngpaste -` first (a fast Homebrew utility that writes
//!   the clipboard image to stdout); if `pngpaste` is missing or fails, falls
//!   back to `osascript`, which is always present on macOS — it extracts the
//!   clipboard as `«class PNGf»`, writes it to a temp file, and we read it back.
//!
//! If no backend is present, the clipboard is empty, or the data is not an
//! image, an `Err(reason)` is sent instead and the drain surfaces a toast.
//!
//! Trigger: Ctrl+V in Chat mode (only when no request is in flight and the mode
//! is not already in a clipboard fetch).  Ctrl+V was chosen because it is the
//! conventional "paste" key; the terminal already delivers text pastes via the
//! bracketed-paste protocol (which koma routes through `handle_paste`), so
//! Ctrl+V would otherwise be a no-op here.

use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use crate::app::state::AppStateRest;

/// Maximum time to wait for the clipboard tool to respond (2 s). Paste should
/// be near-instant; this guards against a stalled `xclip`/`wl-paste`/`osascript`.
const CLIPBOARD_TIMEOUT: Duration = Duration::from_millis(2_000);

/// Spawn a background thread that reads PNG bytes from the OS clipboard and
/// sends the result to `state.rest.clipboard_rx`.
///
/// A fetch is a no-op when one is already in flight (`clipboard_rx` is
/// `Some`). The caller (the Ctrl+V key handler) is responsible for checking
/// this before calling.
///
/// On Linux the thread tries `wl-paste` first (Wayland; environment variable
/// `WAYLAND_DISPLAY` present) then falls back to `xclip` (X11). On macOS it
/// tries `pngpaste` then `osascript`. If no backend is available the thread
/// sends an `Err(...)` describing what to install.
pub fn request_clipboard_image(rest: &mut AppStateRest) {
    // Already fetching — ignore a duplicate Ctrl+V.
    if rest.clipboard_rx.is_some() {
        return;
    }

    let (tx, rx) = mpsc::channel::<Result<Vec<u8>, String>>();
    rest.clipboard_rx = Some(rx);

    std::thread::spawn(move || {
        let result = fetch_clipboard_png();
        // Dropped receiver (app closing) — silently discard.
        let _ = tx.send(result);
    });
}

// ===========================================================================
// Linux backend (Wayland `wl-paste` / X11 `xclip`)
// ===========================================================================

/// Try to read PNG bytes from the clipboard. Returns the raw bytes on success
/// or a human-readable error string on failure. Blocking — runs on a
/// background thread.
#[cfg(target_os = "linux")]
fn fetch_clipboard_png() -> Result<Vec<u8>, String> {
    // Detect Wayland vs X11. `wl-paste` is preferred when a Wayland display is
    // present; otherwise (or on failure) we fall back to `xclip`.
    let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();

    if is_wayland {
        match try_wl_paste() {
            Ok(bytes) if !bytes.is_empty() => return Ok(bytes),
            Ok(_) => return Err("clipboard is empty or contains no PNG image".to_string()),
            Err(_) => {
                // wl-paste not available or failed: fall through to xclip.
            }
        }
    }
    // Try xclip (X11 or Wayland with XWayland).
    match try_xclip() {
        Ok(bytes) if !bytes.is_empty() => Ok(bytes),
        Ok(_) => Err("clipboard is empty or contains no PNG image".to_string()),
        Err(e) => Err(e),
    }
}

/// Run `wl-paste --type image/png` and return its stdout as raw bytes.
#[cfg(target_os = "linux")]
fn try_wl_paste() -> Result<Vec<u8>, String> {
    let child = Command::new("wl-paste")
        .args(["--type", "image/png"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("wl-paste not available: {e}"))?;

    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
        Ok(Ok(out)) if out.status.success() => Ok(out.stdout),
        Ok(Ok(_)) => Err("wl-paste exited with error (no PNG on clipboard)".to_string()),
        Ok(Err(e)) => Err(format!("wl-paste failed: {e}")),
        Err(_) => Err("wl-paste timed out".to_string()),
    }
}

/// Run `xclip -selection clipboard -t image/png -o` and return its stdout.
#[cfg(target_os = "linux")]
fn try_xclip() -> Result<Vec<u8>, String> {
    let child = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-o"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| {
            "no clipboard tool found (install wl-paste for Wayland or xclip for X11)".to_string()
        })?;

    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
        Ok(Ok(out)) if out.status.success() => Ok(out.stdout),
        Ok(Ok(_)) => Err("xclip exited with error (no PNG on clipboard)".to_string()),
        Ok(Err(e)) => Err(format!("xclip failed: {e}")),
        Err(_) => Err("xclip timed out".to_string()),
    }
}

// ===========================================================================
// macOS backend (`pngpaste` → `osascript` fallback)
// ===========================================================================

/// Try to read PNG bytes from the clipboard. Returns the raw bytes on success
/// or a human-readable error string on failure. Blocking — runs on a
/// background thread.
///
/// `pngpaste` (Homebrew: `brew install pngpaste`) is fast and writes the
/// clipboard image straight to stdout, so it is tried first. If it is not
/// installed or fails, we fall back to `osascript`, which ships with every
/// macOS install — it cannot pipe binary to stdout cleanly, so it writes the
/// clipboard PNG to a temp file that we then read back.
#[cfg(target_os = "macos")]
fn fetch_clipboard_png() -> Result<Vec<u8>, String> {
    // Fast path: pngpaste writes the clipboard image to stdout.
    match try_pngpaste() {
        Ok(bytes) if !bytes.is_empty() => return Ok(bytes),
        Ok(_) => {
            // pngpaste ran but produced nothing — treat as "no image" only if
            // pngpaste itself reports success with empty output. Fall through
            // to osascript to double-check via the always-present backend.
        }
        Err(_) => {
            // pngpaste not installed or errored: fall through to osascript.
        }
    }
    // Reliable fallback: osascript is always present on macOS.
    try_osascript()
}

/// Run `pngpaste -` and return its stdout as raw PNG bytes.
///
/// `pngpaste` exits non-zero when there is no image on the clipboard, which we
/// surface as an error so the caller falls back to `osascript`.
#[cfg(target_os = "macos")]
fn try_pngpaste() -> Result<Vec<u8>, String> {
    let child = Command::new("pngpaste")
        .arg("-")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("pngpaste not available: {e}"))?;

    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::Output>>();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
        Ok(Ok(out)) if out.status.success() => Ok(out.stdout),
        Ok(Ok(_)) => Err("pngpaste exited with error (no PNG on clipboard)".to_string()),
        Ok(Err(e)) => Err(format!("pngpaste failed: {e}")),
        Err(_) => Err("pngpaste timed out".to_string()),
    }
}

/// Extract the clipboard PNG via `osascript` and return its bytes.
///
/// AppleScript cannot reliably stream binary data to stdout, so the script
/// fetches `the clipboard as «class PNGf»` and writes it to a temp file using
/// the Standard Additions `write` command. We then read the file back and
/// remove it. The temp path is unique per invocation (PID + nanos) to avoid
/// collisions between concurrent pastes.
#[cfg(target_os = "macos")]
fn try_osascript() -> Result<Vec<u8>, String> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp_path = std::env::temp_dir().join(format!("koma-clip-{}-{}.png", std::process::id(), nanos));
    let tmp_str = tmp_path.to_string_lossy().to_string();

    // AppleScript: read the clipboard as PNG, open the temp file for writing,
    // write the bytes, and always close the file. `quoted form of` would be
    // ideal but the path is a koma-generated temp name with no metacharacters,
    // so a plain quoted POSIX string is safe here.
    let script = format!(
        "set theFile to (POSIX file \"{tmp_str}\")\n\
         set thePNG to (the clipboard as «class PNGf»)\n\
         set fd to open for access theFile with write permission\n\
         try\n\
         \tset eof fd to 0\n\
         \twrite thePNG to fd\n\
         end try\n\
         close access fd"
    );

    let child = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("osascript not available: {e}"))?;

    let (tx, rx) = mpsc::channel::<std::io::Result<std::process::ExitStatus>>();
    std::thread::spawn(move || {
        let mut child = child;
        let _ = tx.send(child.wait());
    });

    let status_ok = match rx.recv_timeout(CLIPBOARD_TIMEOUT) {
        Ok(Ok(status)) => status.success(),
        Ok(Err(e)) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(format!("osascript failed: {e}"));
        }
        Err(_) => {
            let _ = std::fs::remove_file(&tmp_path);
            return Err("osascript timed out".to_string());
        }
    };

    // osascript exits non-zero when the clipboard holds no PNG (the coercion
    // to «class PNGf» fails). Surface that as a clear "no image" error.
    if !status_ok {
        let _ = std::fs::remove_file(&tmp_path);
        return Err("clipboard is empty or contains no PNG image".to_string());
    }

    let bytes = std::fs::read(&tmp_path)
        .map_err(|e| format!("could not read clipboard temp file: {e}"));
    let _ = std::fs::remove_file(&tmp_path);

    match bytes {
        Ok(b) if !b.is_empty() => Ok(b),
        Ok(_) => Err("clipboard is empty or contains no PNG image".to_string()),
        Err(e) => Err(e),
    }
}
