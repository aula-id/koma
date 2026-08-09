//! The `/attach <path>` command: ingest a `.screenshoot/*.png` into the
//! session's pending attachments and insert the `[Image #N]` marker into the
//! composer.

use anyhow::Result;

use crate::app::state::AppState;

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
        let dir = cwd.join(".screenshoot");
        if !dir.is_dir() {
            state.rest.fg_mut().status =
                "no .screenshoot/ directory — use web_screenshot to capture a page first".into();
            return Ok(());
        }
        let mut names: Vec<String> = std::fs::read_dir(&dir)
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.path()
                    .extension()
                    .map(|ext| ext == "png")
                    .unwrap_or(false)
            })
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
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

    // Resolve the path: bare filename → look in `.screenshoot/`; path with
    // `.screenshoot/` prefix → strip it; absolute / relative → use as-is.
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

/// Resolve an `/attach` argument to an absolute path:
/// - bare filename `"shot.png"` → `<cwd>/.screenshoot/shot.png`
/// - prefixed `".screenshoot/shot.png"` → `<cwd>/.screenshoot/shot.png`
/// - absolute/relative path → as-is
fn resolve_screenshoot_arg(cwd: &std::path::Path, arg: &str) -> std::path::PathBuf {
    let p = std::path::Path::new(arg);
    if p.is_absolute() {
        p.to_path_buf()
    } else if arg.starts_with(".screenshoot/") || arg.starts_with("./.screenshoot/") {
        cwd.join(arg)
    } else if !arg.contains('/') && !arg.contains('\\') {
        // Bare filename — look in `.screenshoot/`
        cwd.join(".screenshoot").join(arg)
    } else {
        cwd.join(arg)
    }
}
