//! View – in-app settings dashboard (Settings mode).
//!
//! Two-pane layout: a narrow sidebar lists the [`SETTING_CATEGORIES`]; the
//! detail pane on the right shows all fields for the selected category.  Focus
//! travels left→right (sidebar → detail) and back.  A context-sensitive footer
//! at the bottom shows key hints.
//!
//! Border convention (strict, matches project rules):
//! - Header: `Borders::BOTTOM` only.
//! - Sidebar/detail divider: `Borders::RIGHT` on the sidebar pane.
//! - Footer: plain dim line (no full box anywhere).
//!
//! Layout:
//! ```text
//!  settings
//! ─────────────────────────────────────────────────────────
//! │ Connection  │  API key       sk-or-v1-abc…
//! │ Appearance  │  Model         openai/gpt-oss-120b
//! │ Session     │  Provider      groq
//!               │
//!  ↑/↓ category · →/Enter fields · Esc save & close
//! ```
//!
//! All draft mutation lives in [`app::mode::SettingsState`]; key handling lives
//! in [`controller::input::handle_settings`].

mod utils;
mod providers;
mod pickers;
mod modals;

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Padding, Paragraph},
    Frame,
};
use crate::app::mode::{SETTING_CATEGORIES, SettingField, SettingsState};
use crate::app::state::AppStateRest;
use crate::model::app_config::ThemeMode;
use crate::view::theme::{resolve_accent, Palette};
use providers::{draw_providers, draw_models};
use pickers::draw_role_picker;
use modals::{draw_provider_modal, draw_model_modal};
use utils::truncate;

/// Sidebar column width in terminal columns (includes the RIGHT border char).
const SIDEBAR_W: u16 = 22;

/// Render the settings dashboard for `st` using the given colour `palette`.
///
/// `models_cache` is the on-demand model catalogue and `cache_endpoint` the
/// endpoint it was fetched for (`None` = never fetched). The Models Select modal's
/// omnisearch renders live results only when `cache_endpoint` matches the EDITED
/// provider's endpoint; otherwise it shows `searching models…` (still fetching) or
/// `no models — type an id` (fetched empty).
///
/// All colours flow through `palette` — no hardcoded `Color::` values except
/// the per-accent tint resolved via [`resolve_accent`].
pub fn draw(
    frame: &mut Frame,
    rest: &AppStateRest,
    st: &SettingsState,
    models_cache: &[crate::dto::openrouter::ModelInfo],
    cache_endpoint: Option<&str>,
    palette: &Palette,
) {
    let dark = st.theme == ThemeMode::Dark;

    // Outer vertical zones: header | body | footer.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Min(0),    // sidebar + detail
            Constraint::Length(1), // footer key hints
        ])
        .split(frame.area());

    // --- Header ---
    // "settings" in dim, with a BOTTOM border rule — same idiom as chat.rs.
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled("settings", Style::default().fg(palette.dim)))
            .style(Style::default()),
        header_inner.inner(Margin { horizontal: 2, vertical: 0 }),
    );

    // --- Body: horizontal split into sidebar + detail ---
    let body_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(SIDEBAR_W), // sidebar with RIGHT border as column divider
            Constraint::Min(0),            // detail pane
        ])
        .split(outer[1]);

    // Sidebar block: RIGHT border acts as the column divider.
    let sidebar_block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(palette.dim));
    let sidebar_inner = sidebar_block.inner(body_cols[0]);
    frame.render_widget(sidebar_block, body_cols[0]);

    // Sidebar content: one line per category; inset by 1 col on the left.
    // Group headers are injected whenever the group changes between consecutive
    // categories. Headers are dim, non-selectable; categories are indented under them.
    let sidebar_content = sidebar_inner.inner(Margin { horizontal: 1, vertical: 1 });
    let mut sidebar_lines: Vec<Line> = Vec::new();
    let mut last_group: Option<&str> = None;
    for (i, cat) in SETTING_CATEGORIES.iter().enumerate() {
        if Some(cat.group) != last_group {
            // Spacer before group header (skip before the very first line).
            if last_group.is_some() {
                sidebar_lines.push(Line::from(""));
            }
            sidebar_lines.push(Line::from(vec![
                Span::styled(
                    cat.group,
                    Style::default().fg(palette.dim).add_modifier(Modifier::DIM),
                ),
            ]));
            last_group = Some(cat.group);
        }
        let is_selected = i == st.cat;
        let (marker, color) = if is_selected {
            // Show marker regardless of which pane has focus; dim slightly
            // when focus is in the detail pane to signal the sidebar is passive.
            let c = if st.in_detail {
                palette.dim
            } else {
                palette.accent
            };
            ("› ", c)
        } else {
            ("  ", palette.dim)
        };
        // Indent category name by 2 extra spaces so it sits under its group header.
        sidebar_lines.push(Line::from(vec![
            Span::styled("  ", Style::default().fg(color)),
            Span::styled(marker, Style::default().fg(color)),
            Span::styled(cat.name, Style::default().fg(color)),
        ]));
    }
    frame.render_widget(Paragraph::new(sidebar_lines), sidebar_content);

    // Detail pane: inset by 1 col on each side, 1 row on top.
    let detail_inner = body_cols[1].inner(Margin { horizontal: 2, vertical: 1 });
    let cat_fields = SETTING_CATEGORIES[st.cat].fields;

    // Available width for value column: detail width minus label column (14) minus
    // marker (2).
    let detail_w = detail_inner.width as usize;
    let value_w = detail_w.saturating_sub(16);

    // API Providers / Models Select: custom interactive list screens (no
    // SettingField rows).
    if st.is_providers_category() {
        draw_providers(frame, st, palette, detail_inner);
    } else if st.is_models_category() {
        draw_models(frame, rest, st, palette, detail_inner);
    } else if cat_fields.is_empty() {
        // Stub placeholder for other categories with no fields yet.
        let stub_text = "(stub)";
        frame.render_widget(
            Paragraph::new(stub_text).style(Style::default().fg(palette.dim)),
            detail_inner,
        );
        // Skip the field loop entirely for stub categories.
    } else {

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

    frame.render_widget(Paragraph::new(detail_lines), detail_inner);

    } // end else (non-stub category)

    // --- Footer ---
    // Full-width inverse status bar: background fills the entire footer line
    // edge to edge; text is left-padded by 1 space so it doesn't touch the edge.
    // Context-sensitive: deepest active mode wins (picker → list → editing →
    // field nav → sidebar).
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let on_list_field = st.in_detail
            && !st.is_providers_category()
            && !SETTING_CATEGORIES[st.cat].fields.is_empty()
            && SettingsState::is_path_list(st.current_field());
        // Is the model modal currently in live-omnisearch mode? (Model field, a
        // provider with a non-empty endpoint, non-empty query.)
        let cur_mf = st.mm_current_field();
        let model_search = cur_mf == Some(crate::app::mode::settings::ModelField::Model)
            && st.mm_provider_omnisearchable()
            && st.model_modal.as_ref().map(|m| !m.query.is_empty()).unwrap_or(false);
        let on_route = cur_mf == Some(crate::app::mode::settings::ModelField::Route);
        let on_role  = cur_mf == Some(crate::app::mode::settings::ModelField::Role);
        let role_picker_open = st.mm_role_picker_open();
        let hint = if st.model_modal.is_some() {
            if role_picker_open {
                // The Role checkbox picker owns input while open.
                "↑↓ role · space toggle · enter ok · esc cancel"
            } else if model_search {
                "↑↓ result · enter pick · tab next · esc cancel"
            } else if on_route {
                "↑↓ provider/move · enter pin + next · esc cancel"
            } else if on_role {
                "enter roles · esc cancel"
            } else {
                "↑↓ field · ←→ provider · enter select · esc cancel"
            }
        } else if st.prov_modal.is_some() {
            "↑↓ field · ←→ move/type · enter select · esc cancel"
        } else if st.picker.is_some() {
            "type path · @rel or /abs · ↑/↓ select · Tab descend · Enter pick · Esc cancel"
        } else if st.list_editing {
            "↑/↓ entry · + add · - remove · Enter edit · Esc done"
        } else if st.editing {
            "type to edit · Enter/Esc done"
        } else if st.is_providers_category() && st.in_detail {
            if st.prov_delete_armed {
                "ctrl+x again to CONFIRM delete · any key cancels"
            } else {
                "↑↓ select · + add · ctrl+x delete · esc back"
            }
        } else if st.is_models_category() && st.in_detail {
            if st.model_delete_armed {
                "ctrl+x again to CONFIRM delete · any key cancels"
            } else {
                "↑↓ line · ←→ item · space select · enter open/edit · ctrl+x del · esc back"
            }
        } else if on_list_field {
            "Enter manage list"
        } else if st.in_detail {
            "↑/↓ field · Enter edit/toggle · ←/→ accent · ← back"
        } else {
            "↑/↓ category · →/Enter fields · Esc save & close"
        };
        let bar_style = Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD);
        // Pad the hint with a leading space, then right-pad to the full width so
        // the Paragraph's base style (bar_style) paints the background edge to edge.
        let padded = format!(" {:<width$}", hint, width = footer_rect.width.saturating_sub(1) as usize);
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(padded))).style(bar_style),
            footer_rect,
        );
    }

    // --- FS directory picker overlay ---
    // Mirrors the chat `@` palette: a compact bordered list (the contained-box
    // exception to the flat border convention) showing the live query line and
    // the windowed directory matches. Rendered last so it floats over the panes.
    if let Some(picker) = st.picker.as_ref() {
        const MAX_VIS: usize = crate::app::mode::PICKER_MAX;

        // Query line first, then the matches. The selected match is highlighted.
        let mut rows: Vec<Line> = Vec::new();
        rows.push(Line::from(vec![
            Span::styled("@ ", Style::default().fg(palette.accent)),
            Span::styled(picker.query.as_str(), Style::default().fg(palette.fg)),
            Span::styled("█", Style::default().fg(palette.accent)),
        ]));

        if picker.matches.is_empty() {
            rows.push(Line::from(Span::styled(
                "  (no matching directories)",
                Style::default().fg(palette.dim),
            )));
        } else {
            let sel = picker.sel.min(picker.matches.len().saturating_sub(1));
            // Scrolloff window (persisted offset on rest — SettingsState/PathPicker
            // is rebuilt per client frame).
            let (start, end) = crate::view::scroll::scroll_window(
                &rest.settings_dir_picker_offset,
                sel,
                picker.matches.len(),
                MAX_VIS,
            );
            for (vi, m) in picker.matches[start..end].iter().enumerate() {
                let i = start + vi;
                if i == sel {
                    let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                    rows.push(Line::from(Span::styled(format!(" {m} "), hl)));
                } else {
                    rows.push(Line::from(Span::styled(
                        format!(" {m} "),
                        Style::default().fg(palette.fg),
                    )));
                }
            }
        }

        // Title shows position when more entries exist than fit on screen.
        let title = if picker.matches.len() > MAX_VIS {
            format!(" pick directory {}/{} ", picker.sel + 1, picker.matches.len())
        } else {
            " pick directory ".to_string()
        };

        // Centre a compact box over the body; size to content, clamped.
        let body = outer[1];
        let h = ((rows.len() as u16) + 2).min(body.height.max(3));
        let w = body.width.saturating_sub(4).max(10);
        let x = body.x + (body.width.saturating_sub(w)) / 2;
        let y = body.y + (body.height.saturating_sub(h)) / 2;
        let popup = Rect { x, y, width: w, height: h };

        let block = Block::bordered()
            .border_style(Style::default().fg(palette.dim))
            .title(Span::styled(title, Style::default().fg(palette.dim)))
            .padding(Padding::horizontal(1));
        let inner = block.inner(popup);
        frame.render_widget(Clear, popup);
        frame.render_widget(block, popup);
        frame.render_widget(Paragraph::new(rows), inner);
    }

    // --- Add-provider modal overlay (rendered last, over everything) ---
    if let Some(modal) = st.prov_modal.as_ref() {
        draw_provider_modal(frame, modal, palette, frame.area());
    }

    // --- Add/edit-model modal overlay (rendered last, over everything) ---
    if let Some(modal) = st.model_modal.as_ref() {
        // The Model field is an omnisearch for ANY provider with an endpoint; the
        // Route field stays OpenRouter-only.
        let omni = st.mm_provider_omnisearchable();
        let is_or = st.mm_provider_is_openrouter();
        // Does the cache hold THIS provider's catalogue? (endpoint match)
        let cache_matches = st
            .mm_provider_conn()
            .map(|(ep, _)| cache_endpoint == Some(ep.as_str()))
            .unwrap_or(false);
        draw_model_modal(
            frame, rest, st, modal, omni, is_or, cache_matches, models_cache, palette, frame.area(),
        );

        // Role checkbox picker overlay: a modal-on-modal, drawn LAST so it floats
        // over the model modal it belongs to.
        if let Some(picker) = modal.role_picker.as_ref() {
            draw_role_picker(frame, picker, palette, frame.area());
        }
    }
}
