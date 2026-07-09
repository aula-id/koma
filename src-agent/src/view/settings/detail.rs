//! Detail-pane renderers for the `/settings` dashboard: the generic per-field
//! row list (every category with plain [`SettingField`] rows) and the
//! Appearance category's coolors-style palette-swatch list. Split out of
//! [`super`] (the `settings` view module) for file size — pure code motion.
//! Both are bumped to `pub(super)` (were private) since `draw` (the parent)
//! calls them; no behaviour change.

use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::mode::{SettingField, SettingsState};
use crate::app::state::AppStateRest;
use crate::model::app_config::ThemeMode;
use crate::view::theme::{resolve_accent, Palette};

use super::utils::truncate;

/// Render the plain field-row list for the current category into `detail_inner`
/// (every category except Providers / OAuth / Models Select / Appearance, and
/// only when it has at least one field — the stub/empty case is handled by the
/// caller). Mirrors the original inline loop exactly: PATH LIST fields
/// (Workdir / Allowed dirs) get a label row + one line-wrapped row per entry;
/// every other field gets a single `label   value` row.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_field_list(
    frame: &mut Frame,
    st: &SettingsState,
    palette: &Palette,
    dark: bool,
    cat_fields: &[SettingField],
    detail_inner: Rect,
    detail_w: usize,
    value_w: usize,
) {
    let mut detail_lines: Vec<Line> = Vec::new();
    for (i, &f) in cat_fields.iter().enumerate() {
        let is_selected = st.in_detail && i == st.field;

        // Marker: only shown when detail pane has focus.
        let marker = Span::styled(
            if is_selected { "› " } else { "  " },
            Style::default().fg(palette.accent),
        );

        // Label: left-padded to 14 cols.
        let label_text = format!("{:<14}", f.label());
        let label_color = if is_selected { palette.accent } else { palette.dim };
        let label_span = Span::styled(label_text, Style::default().fg(label_color));

        // PATH LISTS (Workdir / Allowed dirs): a label row, then one
        // line-wrapped row per entry. Each entry hangs under the value
        // column; the highlighted entry (while managing this field) gets a
        // `›` accent marker, the rest are dim. Multiple lines per field, so
        // this is handled before the single-line value logic below.
        if SettingsState::is_path_list(f) {
            let managing = st.list_editing && is_selected;
            // Affordance shown inline with the label when this field is active
            // but not yet being managed (hints how to open it).
            let label_suffix: Vec<Span> = if is_selected && !managing {
                vec![Span::styled("list", Style::default().fg(palette.dim))]
            } else {
                Vec::new()
            };
            let mut header = vec![marker, label_span];
            header.extend(label_suffix);
            detail_lines.push(Line::from(header));

            let entries = st.path_list(f).cloned().unwrap_or_default();
            // Entry rows are indented under the value column; wrap to the
            // remaining width so long absolute paths line-wrap instead of
            // truncating. 4 = 2 (entry marker) + 2 (hanging indent base).
            let entry_w = detail_w.saturating_sub(6).max(1);
            for (ei, entry) in entries.iter().enumerate() {
                let here = managing && ei == st.list_sel;
                let (emark, ecolor) = if here {
                    ("  › ", palette.accent)
                } else {
                    ("    ", palette.dim)
                };
                let wrapped = crate::view::markdown::wrap_spans(
                    &[Span::styled(entry.clone(), Style::default().fg(ecolor))],
                    entry_w,
                );
                if wrapped.is_empty() {
                    detail_lines.push(Line::from(vec![Span::styled(
                        emark,
                        Style::default().fg(ecolor),
                    )]));
                }
                for (wi, vline) in wrapped.into_iter().enumerate() {
                    // First visual line carries the entry marker; continuations
                    // get a 4-col hanging indent so wraps align under it.
                    let prefix = if wi == 0 {
                        Span::styled(emark, Style::default().fg(ecolor))
                    } else {
                        Span::raw("    ")
                    };
                    let mut spans = vec![prefix];
                    spans.extend(vline);
                    detail_lines.push(Line::from(spans));
                }
            }
            continue;
        }

        // Value span(s).
        let value_spans: Vec<Span> = match f {
            SettingField::Theme => {
                let mode_str = match st.theme {
                    ThemeMode::Dark  => "dark",
                    ThemeMode::Light => "light",
                };
                vec![Span::styled(mode_str, Style::default().fg(palette.accent))]
            }
            SettingField::Accent => {
                // Show the accent name coloured in its resolved tint.
                let tint: Color = resolve_accent(&st.accent, dark);
                vec![Span::styled(st.accent.as_str(), Style::default().fg(tint))]
            }
            SettingField::AwarenessEnabled => {
                // Boolean toggle: on/off.
                let v = if st.awareness_enabled { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::ClassifierEnabled => {
                // Boolean toggle: on/off (master switch for the harness).
                let v = if st.classifier_enabled { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::ShortSendEnabled => {
                // Boolean toggle: on/off (kill switch for the token saver).
                let v = if st.short_send_enabled { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::SlidingCache => {
                // Boolean toggle: on/off (on only for providers with a sliding
                // prompt cache, e.g. Anthropic).
                let v = if st.sliding_cache { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::BashSaving => {
                // Boolean toggle: on/off (whether bash/git_operator save output).
                let v = if st.bash_saving { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::InternetMode => {
                // Enum toggle: simple (in-process DDG) vs full (scrapion subprocess).
                let v = st.internet_mode.as_str();
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::AwarenessSource => {
                // Boolean toggle: inherit the session model, or a custom one.
                let v = if st.awareness_inherit {
                    "inherit parent"
                } else {
                    "custom"
                };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::AwarenessModel | SettingField::AwarenessProvider
                if st.awareness_inherit =>
            {
                // Irrelevant while inheriting → dimmed "(inherited)".
                vec![Span::styled("(inherited)", Style::default().fg(palette.dim))]
            }
            _ => {
                // Text field: show draft with optional cursor block.
                let raw: &str = match f {
                    SettingField::ApiKey   => st.api_key.as_str(),
                    SettingField::Model    => st.model.as_str(),
                    SettingField::Provider => {
                        if st.provider.is_empty() {
                            // placeholder shown in dim — handled specially below
                            ""
                        } else {
                            st.provider.as_str()
                        }
                    }
                    SettingField::Name    => st.name.as_str(),
                    // Reached only when source == "custom" (the inherit case
                    // is handled in the arm above).
                    SettingField::AwarenessModel    => st.awareness_model.as_str(),
                    SettingField::AwarenessProvider => st.awareness_provider.as_str(),
                    SettingField::ClassifierModel    => st.classifier_model.as_str(),
                    SettingField::ClassifierProvider => st.classifier_provider.as_str(),
                    // Theme, Accent, the toggles, and the PATH LISTS
                    // (Workdir / AllowedFolders) are handled above; this arm
                    // is unreachable for them.
                    _ => "",
                };
                let editing_here = st.editing && is_selected;
                let truncate_w = if editing_here {
                    value_w.saturating_sub(1)
                } else {
                    value_w
                };
                // Provider placeholder when empty.
                if f == SettingField::Provider && raw.is_empty() && !editing_here {
                    detail_lines.push(Line::from(vec![
                        marker,
                        label_span,
                        Span::styled("default", Style::default().fg(palette.dim)),
                    ]));
                    continue;
                }
                // ApiKey: truncate to max 40 chars.
                let display_raw = if f == SettingField::ApiKey {
                    truncate(raw, truncate_w.min(40))
                } else {
                    truncate(raw, truncate_w)
                };
                let mut shown = display_raw;
                if editing_here {
                    shown.push('█');
                }
                vec![Span::styled(shown, Style::default().fg(palette.fg))]
            }
        };

        let mut spans = vec![marker, label_span];
        spans.extend(value_spans);
        detail_lines.push(Line::from(spans));
    }

    // Appearance short-circuits into `draw_palette_list` (never reaches here), so
    // every category that reaches this fn renders its rows full-height here.
    frame.render_widget(Paragraph::new(detail_lines), detail_inner);
}

/// Render the coolors-style vertical palette list for the Appearance category.
///
/// One titled box per entry in [`crate::view::theme::PALETTES`], stacked top-down:
/// a swatch strip of that palette's role colours built from THAT palette (its
/// registry constructor), NOT the active UI palette. The cursor box
/// (`st.palette_sel`) gets an ACCENT border; every other box a DIM border. The box
/// whose name equals the live `applied` palette (`config.palette`) gets a
/// `· selected` tag in its title — independent of the cursor, so it stays put as
/// the cursor moves and only follows on Enter (apply).
///
/// Boxes are built to the detail inner width (mirroring the markdown code-block box
/// idiom) so they never overflow the pane; the swatch strip is clipped to the inner
/// width on a very narrow terminal.
///
/// Vertical overflow used to be left to the `Paragraph` (which simply clips), but
/// with enough registry entries the boxes overran the detail pane. Instead we
/// window the list around `st.palette_sel` (à la `scroll_window`, keyed off
/// `rest.settings_palette_offset`) and only render the boxes that fit, with a
/// dim `↑/↓ N more` hint line spending any leftover row budget.
pub(super) fn draw_palette_list(
    frame: &mut Frame,
    rest: &AppStateRest,
    st: &SettingsState,
    palette: &Palette,
    applied: &str,
    area: Rect,
) {
    use crate::view::theme::PALETTES;
    // Need room for the borders; bail on a degenerate pane.
    if area.width < 6 || area.height == 0 {
        return;
    }
    let w = area.width as usize;
    let iw = w.saturating_sub(4); // "│ " + content + " │"

    // Swatch role order (structural → semantic); each is painted via the cell
    // BACKGROUND from the ENTRY's palette. `SW` is one swatch block's width.
    const SW: usize = 2;

    // Each entry renders as a fixed 3-row box: top border, swatch row, bottom
    // border (no gap between boxes). Window the registry around the cursor so
    // only the boxes that fit in `area` are ever built.
    const PER_BOX_ROWS: usize = 3;
    let area_h = area.height as usize;
    let visible = (area_h / PER_BOX_ROWS).max(1);
    let (start, end) = crate::view::scroll::scroll_window(
        &rest.settings_palette_offset,
        st.palette_sel,
        PALETTES.len(),
        visible,
    );
    let hidden_above = start;
    let hidden_below = PALETTES.len().saturating_sub(end);
    // Leftover rows (area not evenly divisible by 3) fund the scroll hints, one
    // row apiece — skip a hint rather than overflow the pane if there's no room.
    let leftover = area_h.saturating_sub(visible * PER_BOX_ROWS);

    let mut lines: Vec<Line> = Vec::new();
    if hidden_above > 0 && leftover >= 1 {
        lines.push(Line::from(Span::styled(
            format!(" ↑ {hidden_above} more"),
            Style::default().fg(palette.dim),
        )));
    }
    for (i, (name, build)) in PALETTES.iter().enumerate().take(end).skip(start) {
        let pv = build();
        let is_cursor = i == st.palette_sel;
        let is_applied = *name == applied;
        // Border + title colour: accent for the cursor box, dim otherwise.
        let bstyle =
            Style::default().fg(if is_cursor { palette.accent } else { palette.dim });

        // --- top border: `┌─ [> ]{name}[ · selected] ───┐`, `w` cols wide ---
        let label = if is_applied {
            if is_cursor {
                format!(" > {name} · selected ")
            } else {
                format!(" {name} · selected ")
            }
        } else {
            if is_cursor {
                format!(" > {name} ")
            } else {
                format!(" {name} ")
            }
        };
        let used = 2 /*┌─*/ + label.chars().count();
        let fill = w.saturating_sub(used + 1 /*┐*/);
        lines.push(Line::from(vec![
            Span::styled("┌─".to_string(), bstyle),
            Span::styled(label, bstyle),
            Span::styled(format!("{}┐", "─".repeat(fill)), bstyle),
        ]));

        // --- swatch row: `│ ██ ██ … │`, clipped + right-padded to `iw` ---
        let roles: [Color; 9] = [
            pv.bg, pv.panel, pv.dim, pv.fg, pv.accent, pv.info, pv.success, pv.warn, pv.error,
        ];
        let mut content: Vec<Span> = Vec::new();
        let mut used_w = 0usize;
        for (ri, c) in roles.iter().enumerate() {
            let gap = usize::from(ri > 0); // 1-col gap between swatches
            if used_w + gap + SW > iw {
                break; // never overflow the box inner width
            }
            if gap == 1 {
                content.push(Span::raw(" "));
                used_w += 1;
            }
            content.push(Span::styled(" ".repeat(SW), Style::default().bg(*c)));
            used_w += SW;
        }
        let pad = iw.saturating_sub(used_w);
        if pad > 0 {
            content.push(Span::raw(" ".repeat(pad)));
        }
        let mut row = Vec::with_capacity(content.len() + 2);
        row.push(Span::styled("│ ".to_string(), bstyle));
        row.extend(content);
        row.push(Span::styled(" │".to_string(), bstyle));
        lines.push(Line::from(row));

        // --- bottom border ---
        lines.push(Line::from(Span::styled(
            format!("└{}┘", "─".repeat(w.saturating_sub(2))),
            bstyle,
        )));
    }
    if hidden_below > 0 && leftover >= 2 {
        lines.push(Line::from(Span::styled(
            format!(" ↓ {hidden_below} more"),
            Style::default().fg(palette.dim),
        )));
    }

    frame.render_widget(Paragraph::new(lines), area);
}
