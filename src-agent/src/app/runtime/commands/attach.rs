//! The `/attach <path>` command: ingest a `.screenshoot/*.png` into the
//! session's pending attachments and insert the `[Image #N]` marker into the
//! composer.

use anyhow::Result;

use crate::app::state::AppState;
use crate::model::attachment::{list_screenshoot_pngs, resolve_screenshoot_path};

/// Handle `/attach <path>`: resolve the path against the session cwd, ingest
/// the image into the session's `images/` directory, stage the attachment
/// record, and insert the `[Image #N]` marker at the composer caret.
///
/// No args → list available `.screenshoot/*.png` captures. Missing session →
/// error. Bad path / non-image → error with the system-level message from the
/// ingest core.
pub(super) fn handle_attach(arg: &str, state: &mut AppState) -> Result<()> {
    let arg = arg.trim();
    if arg.is_empty() {
        // List available captures from the project's `.screenshoot/` dir.
        let cwd = state.rest.fg().effective_cwd();
        let names = list_screenshoot_pngs(&cwd);
        if names.is_empty() {
            state.rest.fg_mut().status =
                "no .screenshoot/*.png captures found — use web_screenshot first".into();
        } else {
            let preview: Vec<String> = names.iter().take(8).cloned().collect();
            state.rest.fg_mut().status = format!(
                "captures: {}{}",
                preview.join(", "),
                if names.len() > 8 {
                    format!(" (+{} more)", names.len() - 8)
                } else {
                    String::new()
                }
            );
        }
        return Ok(());
    }

    if state.rest.fg().session.is_none() {
        state.rest.fg_mut().status = "no active session".into();
        return Ok(());
    }

    // Resolve the path via the shared helper: bare filename → look in
    // `.screenshoot/`; prefix stripped internally; absolute paths rejected
    // unless inside `.screenshoot/`.
    let cwd = state.rest.fg().effective_cwd();
    let src_path = resolve_screenshoot_arg(&cwd, arg);

    if !src_path.exists() {
        state.rest.fg_mut().status = format!("attach: not found: {}", src_path.display());
        return Ok(());
    }
    if state
        .rest
        .try_attach_image_path(&src_path.to_string_lossy())
    {
        state.rest.fg_mut().status = format!("attached: {}", src_path.display());
    } else {
        state.rest.fg_mut().status =
            format!("attach: not a recognised image: {}", src_path.display());
    }
    Ok(())
}

/// Resolve an `/attach` argument to an absolute path, normalizing various
/// input forms to a bare `.screenshoot/` name before delegating to the
/// shared helper:
/// - bare filename `"shot.png"` → `resolve_screenshoot_path(cwd, "shot.png")`
/// - prefixed `".screenshoot/shot.png"` → `resolve_screenshoot_path(cwd, "shot.png")`
/// - absolute/relative path outside `.screenshoot/` → returned as-is
fn resolve_screenshoot_arg(cwd: &std::path::Path, arg: &str) -> std::path::PathBuf {
    // Strip common prefixes to extract the bare filename.
    let stripped = arg
        .strip_prefix("./.screenshoot/")
        .or_else(|| arg.strip_prefix(".screenshoot/"))
        .or_else(|| arg.strip_prefix(".screenshoot\\"))
        .unwrap_or(arg);

    // For bare filenames (no path separators), delegate to the shared helper.
    if !stripped.contains('/') && !stripped.contains('\\') {
        if let Some(p) = resolve_screenshoot_path(cwd, stripped) {
            return p;
        }
        // Fall through to cwd/.screenshoot/name even if not yet a file (caller
        // checks existence).
        return cwd.join(".screenshoot").join(stripped);
    }

    // For anything else (relative/absolute), resolve against cwd.
    cwd.join(stripped)
}
