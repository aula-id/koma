//! Paste handler: routes clipboard text to the correct active field.

use crate::app::mode::Mode;
use crate::app::state::AppState;

/// Feed `text` into a SINGLE-LINE field through `sink`, one char at a time,
/// dropping every `\r`/`\n` (and bare CR from CRLF) so a multi-line clipboard
/// can never corrupt a one-line field. Mirrors a normal `Char` keystroke per
/// surviving char, which is the only way to reuse each field's existing
/// active-target resolution (`push_char` step/field logic, `mm_push_char`
/// query-vs-id routing, …).
fn paste_single_line(text: &str, mut sink: impl FnMut(char)) {
    for c in text.chars() {
        if c != '\r' && c != '\n' {
            sink(c);
        }
    }
}

/// Insert pasted text into the active input of the current mode/sub-mode,
/// mirroring how a `Char(c)` keystroke is routed there (same deepest-modal
/// priority), but inserting the WHOLE pasted string at once.
///
/// Multiline fields (the Chat input, the agent prompt Body) keep newlines
/// verbatim; every single-line field (endpoints, keys, names, search queries,
/// filters) strips `\n`/`\r` so a multi-line clipboard can't corrupt it.
/// Contexts with no text field (the role/provider pickers, the session/effort
/// pickers, the loading splash) ignore the paste.
pub fn handle_paste(state: &mut AppState, text: &str) {
    match &mut state.mode {
        Mode::Chat => {
            // Image-path paste: if the WHOLE pasted text is a path to an existing
            // image file, route it through the ingest core — copy it into the
            // session's images/ dir, stage the attachment, and insert an
            // `[Image #N]` marker — instead of dumping the raw path into the input.
            // Non-image / non-path / multi-token pastes fall through to the verbatim
            // text insert below, exactly as before.
            if let Some(path) = image_path_paste(text) {
                if state.rest.try_attach_image_path(&path) {
                    return;
                }
            }
            // Multiline verbatim: '\n' is kept (newline in the input, never a
            // submit). NORMALIZE line endings first so bracketed paste that
            // delivers breaks as CRLF or bare CR still lands as real newlines
            // (a bare `\r` used to be dropped, collapsing the paste onto one line).
            let cleaned = text.replace("\r\n", "\n").replace('\r', "\n");
            for c in cleaned.chars() {
                state.rest.push_char(c);
            }
        }
        Mode::KeyInput(form) => {
            // Step 1 on a fetchable endpoint is the catalogue omnisearch: paste
            // feeds the live query and resets the result highlight, exactly as a
            // typed char does, then arms the on-demand catalogue fetch. Every other
            // field is plain text on `model` / `endpoint` / `api_key` via `push_char`.
            if form.step == 1 && form.is_omnisearchable() {
                paste_single_line(text, |c| {
                    form.query.push(c);
                    form.result_sel = 0;
                });
                let endpoint = form.endpoint.trim().to_string();
                let api_key = form.api_key.trim().to_string();
                state.rest.request_catalogue(&endpoint, &api_key);
            } else {
                paste_single_line(text, |c| form.push_char(c));
            }
        }
        Mode::Settings(s) => {
            // Deepest-modal priority, mirroring `handle_settings`:
            //   role picker (no text field) > model modal > provider modal >
            //   FS path picker > plain text field.
            if s.mm_role_picker_open() {
                // Checkbox overlay — no text entry; swallow the paste.
            } else if s.model_modal.is_some() {
                // `mm_push_char` already routes to the active model-modal field:
                // Name → name, Model → omnisearch query (any provider with an
                // endpoint, resets the result highlight) or raw model id, and
                // ignores Route/Role/buttons.
                paste_single_line(text, |c| s.mm_push_char(c));
                // If that fed the Model omnisearch, prime the on-demand fetch for
                // the edited provider's endpoint (debounced; no-op otherwise).
                if s.mm_current_field()
                    == Some(crate::app::mode::settings::ModelField::Model)
                {
                    if let Some((ep, key)) = s.mm_provider_conn() {
                        state.rest.request_catalogue(&ep, &key);
                    }
                }
            } else if s.prov_modal.is_some() {
                // Add-API-provider modal: `modal_push_char` writes to the active
                // text field (name/endpoint/api_key) and no-ops on the buttons.
                paste_single_line(text, |c| s.modal_push_char(c));
            } else if s.picker.is_some() {
                paste_single_line(text, |c| s.picker_push_char(c));
            } else if s.editing {
                paste_single_line(text, |c| s.push_char(c));
            }
        }
        Mode::Agents(a) => {
            use crate::app::mode::AgentEditField;
            // Deepest-modal priority, mirroring `handle_agents`:
            //   field editor (multiline) > model picker (no text field) >
            //   tool picker (filter) > draft field.
            if let Some((_field, ed)) = a.editor.as_mut() {
                // Full-screen field editor: insert the WHOLE clipboard at the
                // cursor, newlines and all (multi-line aware). NORMALIZE line
                // endings first — bracketed paste often delivers line breaks as
                // CRLF or bare CR, so map both to `\n`; stripping the `\r` (as we
                // used to) would have deleted bare-CR breaks and collapsed the
                // whole paste onto one line.
                let cleaned = text.replace("\r\n", "\n").replace('\r', "\n");
                ed.insert_str(&cleaned);
            } else if a.model_picker.is_some() {
                // Single-select list — no text entry; swallow the paste.
            } else if let Some(p) = a.tool_picker.as_mut() {
                // Tool picker live filter (single-line).
                paste_single_line(text, |c| p.push_filter(c));
            } else if a.editing {
                // Typing into a draft field. The Body is the multiline prompt
                // (newlines kept); every other text field is single-line plain text.
                // (The Model field is a picker, never an edited text field.)
                let body = a.field == AgentEditField::Body;
                for c in text.chars() {
                    if c == '\r' || c == '\n' {
                        if c == '\n' && body {
                            a.newline();
                        }
                        continue;
                    }
                    a.push_char(c);
                }
            }
        }
        Mode::Mcp(m) => {
            // The MCP dashboard has only single-line text fields (name / command /
            // args / env / url), inline-edited; the toggle fields take no text. Feed
            // the paste into the active draft when editing, stripping newlines so a
            // multi-line clipboard can't corrupt a one-line field.
            if m.editing {
                paste_single_line(text, |c| m.push_char(c));
            }
        }
        Mode::SessionPicker(p) => {
            // The `--resume` picker has a live search field: paste feeds the
            // query and re-runs the filter, exactly as a typed char does.
            paste_single_line(text, |c| p.query.push(c));
            p.refilter();
        }
        // No text entry on the effort picker, loading splash, usage dashboard,
        // the message-rewind picker, the session hub, or the quit-confirm overlay
        // — paste is a no-op in all of them.
        Mode::Effort(_)
        | Mode::Loading(_)
        | Mode::Usage(_)
        | Mode::MessageRewind(_)
        | Mode::SessionHub(_)
        | Mode::QuitConfirm(_) => {}
    }
}

/// If the pasted `text` is a path to an existing image file, return the cleaned
/// path as an owned String. Otherwise return `None` (the paste is treated as
/// ordinary text).
///
/// Cleaning steps:
/// 1. Strip leading/trailing quotes (if surrounded by matching `"` or `'`).
/// 2. Strip `file://` prefix if present.
/// 3. Unescape shell backslash-escapes (`\<char>` → `<char>`), so drag-and-drop
///    paths with escaped spaces work (e.g., `Screenshot\ 2026` → `Screenshot 2026`).
///
/// The extension + on-disk existence checks gate the detection:
/// - Image extension must be recognized (cf. `has_image_extension`).
/// - The path must exist as a file on disk.
///
/// This means prose mentioning a filename won't hijack the paste (it won't be a
/// real file), but paths with spaces work as long as they exist.
fn image_path_paste(text: &str) -> Option<String> {
    let mut cleaned = text.trim().to_string();
    
    // Strip surrounding quotes if present (matching pair: "" or '')
    if (cleaned.starts_with('"') && cleaned.ends_with('"'))
        || (cleaned.starts_with('\'') && cleaned.ends_with('\''))
    {
        cleaned = cleaned[1..cleaned.len() - 1].to_string();
    }
    
    // Strip file:// scheme if present
    if let Some(stripped) = cleaned.strip_prefix("file://") {
        cleaned = stripped.to_string();
    }
    
    // Unescape backslash-escapes: replace \<char> with <char>
    let mut unescaped = String::new();
    let mut chars = cleaned.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                unescaped.push(next);
            } else {
                unescaped.push(c);
            }
        } else {
            unescaped.push(c);
        }
    }
    cleaned = unescaped;
    
    if cleaned.is_empty() {
        return None;
    }
    
    if !crate::model::attachment::has_image_extension(&cleaned) {
        return None;
    }
    
    if std::path::Path::new(&cleaned).is_file() {
        Some(cleaned)
    } else {
        None
    }
}
