//! View – first-run setup WIZARD (KeyInput mode).
//!
//! A themed, 2-step, provider-agnostic setup screen shown when the app has no
//! usable provider connection (first run, or an explicit reconfigure).
//!
//! Steps:
//! - **0 — connection:** `Endpoint` (any OpenAI-compatible base URL) + `API key`.
//! - **1 — model:** `Model` id. For any non-empty endpoint this is a LIVE search
//!   over the on-demand model catalogue (search line + gray rule + windowed results
//!   dropdown, mirroring `settings::draw_model_modal`); for a blank endpoint it
//!   stays a plain text box. Branch is keyed on [`KeyInputForm::is_omnisearchable`].
//!
//! Layout: a single left-aligned block of BLOCK_W columns, horizontally centred
//! in the frame and placed in the upper portion (≈25 % down). Every element
//! within the block shares the same left edge, so nothing is independently
//! centred. Label column is LABEL_W cols, right-aligned; a 2-space gap separates
//! it from the value column. An underline rule (`─`) sits directly below every
//! field (accent when active, dim when inactive) as the input affordance.
//!
//! Purely presentational: field editing / step transitions live in
//! [`app::mode::KeyInputForm`]; the finish / cancel actions are returned by
//! [`controller::input::handle_key_input`].

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::mode::{filter_models, KeyInputForm};
use crate::app::state::AppStateRest;
use crate::dto::openrouter::ModelInfo;
use crate::view::theme::Palette;

/// Maximum number of catalogue rows shown in the step-1 omnisearch dropdown.
const RESULTS_MAX: usize = 8;

/// Total width (chars) of the content block. Clamped to the available area.
const BLOCK_W: u16 = 58;
/// Width (chars) of the right-aligned label column. Two trailing spaces separate
/// label from value.
const LABEL_W: usize = 9;
/// Gap between label column and value column (spaces).
const GAP: usize = 2;
/// Cursor block glyph appended to the active field's value.
const CURSOR: char = '\u{2588}'; // █

// ── helpers ──────────────────────────────────────────────────────────────────

/// Compute the left-edge x coordinate that centres `block_w` inside `frame_w`.
fn block_x(frame_w: u16, block_w: u16) -> u16 {
    frame_w.saturating_sub(block_w) / 2
}

/// Build a label+value line for a form field.
///
/// The label is right-aligned in LABEL_W cols; a GAP-space separator is
/// appended; then the value text (with an optional trailing cursor block when
/// `active`). Label colour: `palette.accent` when active, else `palette.dim`.
/// Value colour: `palette.fg` when active, else `palette.dim`.
fn field_line<'a>(label: &'a str, value: &str, active: bool, palette: &Palette) -> Line<'a> {
    let (label_color, value_color) = if active {
        (palette.accent, palette.fg)
    } else {
        (palette.dim, palette.dim)
    };
    let mut shown = value.to_string();
    if active {
        shown.push(CURSOR);
    }
    Line::from(vec![
        Span::styled(
            format!("{label:>LABEL_W$}{:GAP$}", ""),
            Style::default().fg(label_color),
        ),
        Span::styled(shown, Style::default().fg(value_color)),
    ])
}

/// Build the underline rule that sits directly below a field row.
///
/// The rule spans the value column only (offset by LABEL_W + GAP spaces).
/// `rule_w` is the available width for the rule (value column width).
/// Colour: `palette.accent` when the field is active, else `palette.dim`.
fn rule_line(rule_w: usize, active: bool, palette: &Palette) -> Line<'static> {
    let color = if active { palette.accent } else { palette.dim };
    Line::from(vec![
        Span::raw(format!("{:width$}", "", width = LABEL_W + GAP)),
        Span::styled("\u{2500}".repeat(rule_w.max(1)), Style::default().fg(color)),
    ])
}

/// Build a dim hint line indented to the value column (e.g. an example model id).
fn hint_line(text: &'static str, palette: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{:width$}", "", width = LABEL_W + GAP)),
        Span::styled(text, Style::default().fg(palette.dim)),
    ])
}

/// Build a dim status/hint line for the omnisearch dropdown, indented by `indent`
/// columns (the value column start). Used for `fetching models…`, the empty-query
/// hint, and the no-matches message.
fn dim_indented(text: &str, indent: usize, palette: &Palette) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("{:indent$}", "")),
        Span::styled(text.to_string(), Style::default().fg(palette.dim)),
    ])
}

/// Truncate `s` to at most `max` chars, appending `…` when cut.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        chars[..max.saturating_sub(1)].iter().collect::<String>() + "\u{2026}"
    }
}

/// Build one omnisearch result row for catalogue model `info`, indented to the
/// value column. The selected row gets the inverse `sel_fg`/`sel_bg` highlight
/// spanning the value column width; unselected rows show the `id` in `fg` and the
/// optional display `name` trailing in `dim`. `val_w` is the value column width.
fn result_line(
    info: &ModelInfo,
    selected: bool,
    indent: usize,
    val_w: usize,
    palette: &Palette,
) -> Line<'static> {
    let id = info.id.clone();
    let name = info.name.as_deref().unwrap_or("");
    let pad = Span::raw(format!("{:indent$}", ""));
    if selected {
        // Inverse-highlight the whole value column so the cursor row is obvious.
        let text = if name.is_empty() {
            format!(" {id} ")
        } else {
            format!(" {id}  {name} ")
        };
        let text = truncate(&text, val_w.max(1));
        Line::from(vec![
            pad,
            Span::styled(text, Style::default().fg(palette.sel_fg).bg(palette.sel_bg)),
        ])
    } else {
        let id_disp = truncate(&id, val_w.saturating_sub(2).max(1));
        let mut spans = vec![pad, Span::styled(id_disp, Style::default().fg(palette.fg))];
        if !name.is_empty() {
            let used = id.chars().count();
            let rem = val_w.saturating_sub(used + 2);
            if rem > 1 {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    truncate(name, rem),
                    Style::default().fg(palette.dim),
                ));
            }
        }
        Line::from(spans)
    }
}

// ── main draw ────────────────────────────────────────────────────────────────

/// Render the setup wizard for `form` using the given colour `palette`.
///
/// `models_cache` is the on-demand model catalogue and `cache_endpoint` the
/// endpoint it was fetched for (`None` = never fetched). Step 1's live search
/// renders results only when `cache_endpoint` matches the ENTERED endpoint;
/// otherwise it shows a dim `searching models…` (still fetching) or `no models —
/// type an id` (fetched empty). Ignored for a blank endpoint (the Model field is a
/// plain text box there).
pub fn draw(
    frame: &mut Frame,
    rest: &AppStateRest,
    form: &KeyInputForm,
    models_cache: &[ModelInfo],
    cache_endpoint: Option<&str>,
    palette: &Palette,
) {
    let area = frame.area();

    // ── Vertical layout ──────────────────────────────────────────────────────
    // A top spacer pushes the block to ≈25 % down; the block body is a Min
    // region so it expands to hold all rows; a dim footer is pinned to the
    // bottom. An explicit bottom margin prevents the footer from touching the
    // terminal edge.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(25), // top spacer → block starts at ~25 %
            Constraint::Min(1),         // block body (title + subtitle + step + fields)
            Constraint::Length(1),      // footer key hints
            Constraint::Length(1),      // bottom margin
        ])
        .split(area);

    // ── Block geometry ───────────────────────────────────────────────────────
    // Clamp block width to the available frame width; compute the left edge so
    // the block is horizontally centred. Every widget in the block is rendered
    // into a 1-row-tall Rect at (block_x, row_y, block_w, 1).
    let block_w = BLOCK_W.min(area.width);
    let bx = block_x(area.width, block_w);

    // The block body region starts at chunks[1].y. We render rows top-down,
    // incrementing `row` for each line.
    let body_y = chunks[1].y;
    let body_h = chunks[1].height;

    // ── Helper: render one line into the block at row offset `row` ───────────
    // Returns immediately when the row would be out of the body region.
    let mut row: u16 = 0;
    let render_line = |frame: &mut Frame, line: Line, r: u16| {
        if r >= body_h {
            return;
        }
        let rect = Rect {
            x: bx,
            y: body_y + r,
            width: block_w,
            height: 1,
        };
        frame.render_widget(Paragraph::new(line), rect);
    };

    // Row 0: "koma" in accent, left-aligned at the block's left edge.
    render_line(
        frame,
        Line::from(Span::styled("koma", Style::default().fg(palette.accent))),
        row,
    );
    row += 1;

    // Row 1: dim subtitle.
    render_line(
        frame,
        Line::from(Span::styled(
            "first-time setup \u{00b7} change anything later in /settings",
            Style::default().fg(palette.dim),
        )),
        row,
    );
    row += 1;

    // Row 2: blank.
    row += 1;

    // Row 3: "step N / 2 · <name>" — step number in accent, rest dim.
    let (step_num, step_name) = if form.step == 0 {
        ("1", "connection")
    } else {
        ("2", "model")
    };
    render_line(
        frame,
        Line::from(vec![
            Span::styled("step ", Style::default().fg(palette.dim)),
            Span::styled(step_num, Style::default().fg(palette.accent)),
            Span::styled(
                format!(" / 2 \u{00b7} {step_name}"),
                Style::default().fg(palette.dim),
            ),
        ]),
        row,
    );
    row += 1;

    // Row 4: blank.
    row += 1;

    // ── Form fields ──────────────────────────────────────────────────────────
    // The value column starts at offset LABEL_W + GAP from the block's left
    // edge; the rule spans from there to the block's right edge.
    let value_col_start = LABEL_W + GAP;
    let rule_w = (block_w as usize).saturating_sub(value_col_start);

    if form.step == 0 {
        // Step 0: Endpoint + API key fields.
        let endpoint_active = form.field == 0;
        let key_active = form.field == 1;

        // Endpoint field row.
        render_line(
            frame,
            field_line("Endpoint", &form.endpoint, endpoint_active, palette),
            row,
        );
        row += 1;
        // Endpoint underline rule.
        render_line(frame, rule_line(rule_w, endpoint_active, palette), row);
        row += 1;

        // Blank spacer between fields.
        row += 1;

        // API key field row.
        render_line(
            frame,
            field_line("API key", &form.api_key, key_active, palette),
            row,
        );
        row += 1;
        // API key underline rule.
        render_line(frame, rule_line(rule_w, key_active, palette), row);
        // row not needed after last field
    } else if form.is_omnisearchable() {
        // Step 1 (fetchable endpoint): the Model field becomes a LIVE catalogue
        // search — the query as the input value, the accent underline, and a
        // windowed results dropdown below (mirrors `settings::draw_model_modal`).
        //
        // Search input row: label "Model" + the live query + cursor block.
        render_line(frame, field_line("Model", &form.query, true, palette), row);
        row += 1;
        // Underline rule (active accent, same as every active field on the form).
        render_line(frame, rule_line(rule_w, true, palette), row);
        row += 1;

        // The catalogue is fetched ON DEMAND for the entered endpoint. Trust
        // `models_cache` only when it was fetched for THIS endpoint; otherwise the
        // fetch is still in flight → `searching models…`. Once it lands, an empty
        // catalogue is terminal → `no models — type an id` (the raw-query fallback
        // still lets Enter finish).
        let indent = LABEL_W + GAP;
        let cache_matches = cache_endpoint == Some(form.endpoint.trim());
        if !cache_matches {
            render_line(
                frame,
                dim_indented("searching models\u{2026}", indent, palette),
                row,
            );
        } else {
            // Show the "type to search" hint above the list while the query is empty.
            if form.query.trim().is_empty() {
                render_line(
                    frame,
                    dim_indented("type to search models\u{2026}", indent, palette),
                    row,
                );
                row += 1;
            }
            let results = filter_models(models_cache, &form.query);
            if results.is_empty() {
                render_line(
                    frame,
                    dim_indented("no models \u{2014} type an id", indent, palette),
                    row,
                );
            } else {
                let sel = form.result_sel.min(results.len() - 1);
                // Scrolloff window (persisted offset on rest — KeyInputForm is
                // rebuilt per client frame).
                let (start, end) = crate::view::scroll::scroll_window(
                    &rest.key_input_results_offset,
                    sel,
                    results.len(),
                    RESULTS_MAX,
                );
                for (vi, &mi) in results[start..end].iter().enumerate() {
                    let i = start + vi;
                    let info = &models_cache[mi];
                    render_line(
                        frame,
                        result_line(info, i == sel, indent, rule_w, palette),
                        row,
                    );
                    row += 1;
                }
            }
        }
    } else {
        // Step 1 (blank endpoint): plain editable Model id with a bottom rule.
        render_line(frame, field_line("Model", &form.model, true, palette), row);
        row += 1;
        // Model underline rule (always active accent on this step).
        render_line(frame, rule_line(rule_w, true, palette), row);
        row += 1;
        // Example hint indented to the value column.
        render_line(frame, hint_line("e.g. koma/apple or openai/…", palette), row);
    }

    // ── Footer ───────────────────────────────────────────────────────────────
    // Pinned to the bottom of the frame, dim, centred.
    let footer_text = if form.step == 0 {
        "tab/\u{2191}\u{2193} switch \u{00b7} enter next \u{00b7} esc cancel \u{00b7} ctrl+c quit"
    } else if form.is_omnisearchable() {
        // Step 1 search mode: ↑↓ navigates the catalogue results.
        "\u{2191}\u{2193} pick \u{00b7} enter finish \u{00b7} esc back \u{00b7} ctrl+c quit"
    } else {
        "enter finish \u{00b7} esc back \u{00b7} ctrl+c quit"
    };
    let footer = Paragraph::new(Line::from(Span::styled(
        footer_text,
        Style::default().fg(palette.dim),
    )));
    // Centre the footer text inside chunks[2].
    let footer_w = footer_text.chars().count() as u16;
    let footer_x = chunks[2]
        .x
        .saturating_add(chunks[2].width.saturating_sub(footer_w) / 2);
    let footer_rect = Rect {
        x: footer_x,
        y: chunks[2].y,
        width: footer_w.min(chunks[2].width),
        height: 1,
    };
    frame.render_widget(footer, footer_rect);
}
