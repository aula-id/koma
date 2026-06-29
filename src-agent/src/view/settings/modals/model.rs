use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Clear, Paragraph},
    Frame,
};
use crate::app::mode::SettingsState;
use crate::app::mode::settings::{ModelModal, ModelField, ModelRole};
use crate::app::mode::filter_models;
use crate::dto::openrouter::ModelInfo;
use crate::view::theme::Palette;
use super::super::utils::{price_per_million, truncate};

/// Render the add/edit-model modal overlay with a dimmed backdrop.
///
/// Mirrors [`draw_provider_modal`] (backdrop dim + `Clear` + bordered accent
/// box), but is taller because the Model field hosts a live omnisearch results
/// list whenever the chosen provider has an endpoint to search.
///
/// - `omni` = `st.mm_provider_omnisearchable()` — the Model field is the live
///   omnisearch (any provider with a non-empty endpoint), not a plain text box.
/// - `is_or` = `st.mm_provider_is_openrouter()` — gates the Route upstream-pin
///   section (OpenRouter-only).
/// - `cache_matches` — whether `cache` was fetched for THIS provider's endpoint.
///   When false the results area shows `searching models…` (still fetching);
///   when true but the filter is empty it shows `no models — type an id`.
#[allow(clippy::too_many_arguments)]
pub(in crate::view::settings) fn draw_model_modal(
    frame: &mut Frame,
    st: &SettingsState,
    modal: &ModelModal,
    omni: bool,
    is_or: bool,
    cache_matches: bool,
    cache: &[ModelInfo],
    palette: &Palette,
    area: Rect,
) {
    // Taller than the provider modal: it hosts a results list.
    // OpenRouter mode adds a separate readout row + search field + rule (2 extra).
    // Edit mode gets one further extra row for the Role field; OpenRouter +
    // model-selected adds a Route label row above the (now selectable) options
    // list. Sized for the taller layout so the 8-row options list never clips.
    //
    // When the search query is EMPTY and a model is selected, the options list
    // renders instead of the results dropdown (they're mutually exclusive). Sized
    // for the taller of the two: a Route label + options header + up to 8 rows + a
    // "+N more" line, which is why this base is generous enough that 8 rows never
    // clip even with the extra Route label row.
    const MODAL_W: u16 = 60;
    const MODAL_H_BASE: u16 = 23;
    let modal_h = if modal.is_edit() { MODAL_H_BASE + 1 } else { MODAL_H_BASE };
    let w = MODAL_W.min(area.width.saturating_sub(2));
    let h = modal_h.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(h)) / 2;
    let popup = Rect { x, y, width: w, height: h };

    // Dim everything outside the modal (preserve the bg-reset fix).
    {
        let buf = frame.buffer_mut();
        for cy in area.top()..area.bottom() {
            for cx in area.left()..area.right() {
                if cx >= popup.x && cx < popup.right() && cy >= popup.y && cy < popup.bottom() {
                    continue;
                }
                buf[(cx, cy)].set_fg(palette.dim).set_bg(Color::Reset);
            }
        }
    }

    let title = if modal.editing_idx.is_some() {
        " Edit model "
    } else {
        " Add model "
    };
    let modal_block = Block::bordered()
        .border_style(Style::default().fg(palette.accent))
        .title(Span::styled(title, Style::default().fg(palette.accent)));
    let inner = modal_block.inner(popup);

    frame.render_widget(Clear, popup);
    frame.render_widget(modal_block, popup);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let label_w = 10usize;
    let val_w   = (inner.width as usize).saturating_sub(label_w + 1).max(4);
    let mut lines: Vec<Line> = Vec::new();

    // Resolve the focused field through the computed field list (no hardcoded
    // indices): `focused(f)` is true when the modal's `field` cursor points at
    // `f` in the current layout. Name/Provider/Model are always the first three.
    let fields = st.model_modal_fields();
    let focused = |f: ModelField| fields.get(modal.field).copied() == Some(f);

    // Row: Name.
    {
        let active = focused(ModelField::Name);
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(format!("{:<width$}", "Name", width = label_w), Style::default().fg(lc));
        let mut val = truncate(&modal.name, val_w.saturating_sub(1));
        if active { val.push('\u{2588}'); }
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![label, Span::styled(val, Style::default().fg(vc))]));
    }

    // Row: Provider toggle.
    {
        let active = focused(ModelField::Provider);
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(format!("{:<width$}", "Provider", width = label_w), Style::default().fg(lc));
        let prov_name = st
            .providers
            .get(modal.provider_idx)
            .map(|p| if p.name.is_empty() { "\u{2014}" } else { p.name.as_str() });
        let toggle_text = match prov_name {
            Some(n) => format!("\u{2039} {} \u{203a}", n),
            None    => "\u{2039} (no providers) \u{203a}".to_string(),
        };
        let tc = if active { palette.accent } else { palette.dim };
        lines.push(Line::from(vec![label, Span::styled(toggle_text, Style::default().fg(tc))]));
    }

    // Row(s): Model.
    // Omnisearch layout (any provider with an endpoint):
    //   1. Read-only selected model readout  (label "Model" + model_id / dim placeholder, NO cursor)
    //   2. Search input line                 (indented to value column; query text + cursor when focused)
    //   3. Gray ─ bottom rule                (dim, spans value column width)
    //   4. Results dropdown / state          (when query is non-empty: searching / results / no models)
    //   5. Route label + selectable options  (OpenRouter only, when query is empty + a model is selected)
    // Plain (blank-endpoint) layout:
    //   1. Plain editable model id           (label "Model" + model_id text + cursor when focused)
    //   2. Gray ─ bottom rule
    {
        let active = focused(ModelField::Model);
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(format!("{:<width$}", "Model", width = label_w), Style::default().fg(lc));

        if omni {
            // --- 1. Selected model readout (read-only, no cursor ever) ---
            if modal.model_id.is_empty() {
                lines.push(Line::from(vec![
                    label,
                    Span::styled("(none selected)", Style::default().fg(palette.dim)),
                ]));
            } else {
                let readout = truncate(&modal.model_id, val_w);
                lines.push(Line::from(vec![
                    label,
                    Span::styled(readout, Style::default().fg(palette.fg)),
                ]));
            }

            // --- 2. Search input line (indented to value column) ---
            {
                let indent = Span::raw(" ".repeat(label_w));
                let search_text = if modal.query.is_empty() {
                    let mut ph = "type to search models\u{2026}".to_string();
                    if active { ph.push('\u{2588}'); }
                    Span::styled(ph, Style::default().fg(palette.dim))
                } else {
                    let mut q = truncate(&modal.query, val_w.saturating_sub(1));
                    if active { q.push('\u{2588}'); }
                    Span::styled(q, Style::default().fg(palette.fg))
                };
                lines.push(Line::from(vec![indent, search_text]));
            }

            // --- 3. Gray bottom rule beneath the search field ---
            {
                let rule_w = val_w.max(1);
                let rule_str = "\u{2500}".repeat(rule_w); // ─ repeated
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(label_w)),
                    Span::styled(rule_str, Style::default().fg(palette.dim)),
                ]));
            }

            // --- 4. Results dropdown / fetch-state (only when query is non-empty) ---
            // The catalogue is fetched on demand for this provider's endpoint.
            // Until the cache holds THIS endpoint (`cache_matches`), show a dim
            // `searching models…`. Once it does: the dropdown, or a dim
            // `no models — type an id` when the (terminal) catalogue is empty / the
            // query matches nothing (the raw-query fallback still lets Enter commit).
            if !modal.query.is_empty() {
                const MAX_VIS: usize = 8;
                let results = if cache_matches {
                    filter_models(cache, &modal.query)
                } else {
                    Vec::new()
                };
                if !cache_matches {
                    lines.push(Line::from(Span::styled(
                        "  searching models\u{2026}",
                        Style::default().fg(palette.dim),
                    )));
                } else if results.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "  no models \u{2014} type an id",
                        Style::default().fg(palette.dim),
                    )));
                } else {
                    let sel = modal.result_sel.min(results.len().saturating_sub(1));
                    let start = if sel < MAX_VIS { 0 } else { sel + 1 - MAX_VIS };
                    let end = (start + MAX_VIS).min(results.len());
                    let row_w = inner.width as usize;
                    for (vi, &mi) in results[start..end].iter().enumerate() {
                        let i = start + vi;
                        let info = &cache[mi];
                        let id = info.id.clone();
                        let name = info.name.as_deref().unwrap_or("");
                        if i == sel {
                            let text = if name.is_empty() {
                                format!(" {id} ")
                            } else {
                                format!(" {id}  {name} ")
                            };
                            let text = truncate(&text, row_w);
                            lines.push(Line::from(Span::styled(
                                text,
                                Style::default().fg(palette.sel_fg).bg(palette.sel_bg),
                            )));
                        } else {
                            let id_disp = truncate(&id, row_w.saturating_sub(2));
                            let mut spans = vec![
                                Span::raw(" "),
                                Span::styled(id_disp, Style::default().fg(palette.fg)),
                            ];
                            if !name.is_empty() {
                                let used = 1 + id.chars().count();
                                let rem = row_w.saturating_sub(used + 2);
                                if rem > 1 {
                                    let n = truncate(name, rem);
                                    spans.push(Span::raw("  "));
                                    spans.push(Span::styled(n, Style::default().fg(palette.dim)));
                                }
                            }
                            lines.push(Line::from(spans));
                        }
                    }
                }
            }

            // --- 5. Route field: label row + selectable options list (OpenRouter
            //        only — the upstream-pin list is OpenRouter-specific). Shown when
            //        the search query is empty so it never stacks under the results
            //        dropdown (the two are mutually exclusive). The Route field only
            //        exists once a model is selected; until then the same area shows
            //        loading / hint states. ---
            if is_or && modal.query.is_empty() {
                let row_w = inner.width as usize;
                let route_active = focused(ModelField::Route);

                if !modal.model_id.is_empty() {
                    // Route label row: shows the committed choice (Auto / pinned
                    // provider name). Accent when the Route field is focused.
                    let lc = if route_active { palette.accent } else { palette.dim };
                    let rl = Span::styled(
                        format!("{:<width$}", "Route", width = label_w),
                        Style::default().fg(lc),
                    );
                    let choice = match modal.route.as_deref() {
                        Some(name) if !name.is_empty() => name.to_string(),
                        _ => "Auto (OpenRouter routes)".to_string(),
                    };
                    let vc = if route_active { palette.fg } else { palette.dim };
                    lines.push(Line::from(vec![
                        rl,
                        Span::styled(truncate(&choice, val_w), Style::default().fg(vc)),
                    ]));
                }

                if modal.endpoints_loading {
                    lines.push(Line::from(Span::styled(
                        "loading routes\u{2026}",
                        Style::default().fg(palette.dim),
                    )));
                } else if let Some(eps) = modal.endpoints.as_ref() {
                    if eps.is_empty() {
                        lines.push(Line::from(Span::styled(
                            "no routes for this model",
                            Style::default().fg(palette.dim),
                        )));
                    } else {
                        // The Route option list: row 0 = Auto, rows 1..=N = each
                        // endpoint. `option_count` and the option `sel`/`pinned`
                        // indices line up with the input-layer route handling.
                        let option_count = 1 + eps.len();
                        let sel = modal.route_sel.min(option_count - 1);
                        // Which option is the committed route? Auto (0) when
                        // `route` is None, else the endpoint whose name matches.
                        let pinned: usize = match modal.route.as_deref() {
                            None => 0,
                            Some(name) => eps
                                .iter()
                                .position(|ep| {
                                    ep.provider_name
                                        .as_deref()
                                        .filter(|n| !n.is_empty())
                                        .or(ep.name.as_deref().filter(|n| !n.is_empty()))
                                        == Some(name)
                                })
                                .map(|i| i + 1)
                                .unwrap_or(0),
                        };

                        // Render Auto + up to 8 endpoint rows, windowed to keep
                        // `sel` visible while the Route field is focused.
                        const MAX_EP: usize = 8;
                        // Build the full option label list first (index 0 = Auto).
                        let mut opt_labels: Vec<String> = Vec::with_capacity(option_count);
                        opt_labels.push("Auto (OpenRouter routes)".to_string());
                        for ep in eps.iter() {
                            let name = ep
                                .provider_name
                                .as_deref()
                                .filter(|n| !n.is_empty())
                                .or(ep.name.as_deref().filter(|n| !n.is_empty()))
                                .unwrap_or("\u{2014}");
                            let (prompt, completion) = ep
                                .pricing
                                .as_ref()
                                .map(|p| (p.prompt.as_ref(), p.completion.as_ref()))
                                .unwrap_or((None, None));
                            let price = format!(
                                "{}/{}",
                                price_per_million(prompt),
                                price_per_million(completion),
                            );
                            let uptime = ep
                                .uptime_last_30m
                                .map(|v| format!("{v:.0}%"))
                                .unwrap_or_default();
                            // name left-padded to ~14, then price, then uptime.
                            opt_labels.push(format!("{name:<14} {price}  {uptime}"));
                        }

                        // Window of MAX_EP+1 rows (Auto always counts as a row).
                        const VIS: usize = MAX_EP + 1;
                        let start = if !route_active || sel < VIS {
                            0
                        } else {
                            sel + 1 - VIS
                        };
                        let end = (start + VIS).min(option_count);
                        for (i, label) in opt_labels
                            .iter()
                            .enumerate()
                            .take(end)
                            .skip(start)
                        {
                            // Persistent marker on the committed route regardless
                            // of focus, so the pin is always visible.
                            let marker = if i == pinned { "\u{2023} " } else { "  " };
                            let text = truncate(
                                &format!("{marker}{label}"),
                                row_w,
                            );
                            let style = if route_active && i == sel {
                                // Focused highlight on the cursor row.
                                Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
                            } else if i == pinned {
                                // Committed route stands out in accent even when
                                // focus is elsewhere.
                                Style::default().fg(palette.accent)
                            } else {
                                Style::default().fg(palette.fg)
                            };
                            lines.push(Line::from(Span::styled(text, style)));
                        }
                        if end < option_count {
                            lines.push(Line::from(Span::styled(
                                format!("+{} more", option_count - end),
                                Style::default().fg(palette.dim),
                            )));
                        }
                    }
                } else if !modal.model_id.is_empty() {
                    // Model set but endpoints not loaded yet (e.g. a fetch failed
                    // to even start): a neutral hint rather than a blank gap.
                    lines.push(Line::from(Span::styled(
                        "loading routes\u{2026}",
                        Style::default().fg(palette.dim),
                    )));
                } else {
                    // No model selected yet.
                    lines.push(Line::from(Span::styled(
                        "pick a model to see providers",
                        Style::default().fg(palette.dim),
                    )));
                }
            }
        } else {
            // Non-OpenRouter: plain editable model id with a bottom rule.
            let mut val = truncate(&modal.model_id, val_w.saturating_sub(1));
            if active { val.push('\u{2588}'); }
            let vc = if active { palette.fg } else { palette.dim };
            lines.push(Line::from(vec![label, Span::styled(val, Style::default().fg(vc))]));

            // Gray bottom rule (consistent input affordance).
            let rule_w = val_w.max(1);
            let rule_str = "\u{2500}".repeat(rule_w);
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(label_w)),
                Span::styled(rule_str, Style::default().fg(palette.dim)),
            ]));
        }
    }

    // Row: Role readout (edit mode only). A single labelled summary line — the
    // comma-joined assigned role labels, or "none". Enter on this field opens the
    // Role checkbox picker overlay (the actual multi-select UI). Accent label +
    // fg value when the Role field is focused; dim otherwise.
    if modal.is_edit() {
        let active = focused(ModelField::Role);
        let lc     = if active { palette.accent } else { palette.dim };
        let label  = Span::styled(
            format!("{:<width$}", "Role", width = label_w),
            Style::default().fg(lc),
        );
        let value = if modal.roles.is_empty() {
            "none".to_string()
        } else {
            modal
                .roles
                .iter()
                .map(|r: &ModelRole| r.label())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![
            label,
            Span::styled(truncate(&value, val_w), Style::default().fg(vc)),
        ]));
    }

    // Blank line before the buttons.
    lines.push(Line::from(""));

    // Button row: `[ Save ]  [ Save session ]  [ Cancel ]` centered together.
    // Only the chip text carries the highlight bg; inter-chip spacing uses plain
    // style so the background does not bleed across the modal width.
    let save_text    = "[ Save ]";
    let session_text = "[ Save session ]";
    let cancel_text  = "[ Cancel ]";
    let gap          = "  ";
    let group_len    = save_text.len() + gap.len() + session_text.len() + gap.len() + cancel_text.len();
    let inner_w      = inner.width as usize;
    let pad_left     = inner_w.saturating_sub(group_len) / 2;
    let pad_right    = inner_w.saturating_sub(group_len).saturating_sub(pad_left);
    let save_style = if focused(ModelField::Save) {
        Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
    } else {
        Style::default().fg(palette.accent)
    };
    let session_style = if focused(ModelField::SaveSession) {
        Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
    } else {
        Style::default().fg(palette.accent)
    };
    let cancel_style = if focused(ModelField::Cancel) {
        Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
    } else {
        Style::default().fg(palette.accent)
    };
    lines.push(Line::from(vec![
        Span::raw(" ".repeat(pad_left)),
        Span::styled(save_text, save_style),
        Span::raw(gap),
        Span::styled(session_text, session_style),
        Span::raw(gap),
        Span::styled(cancel_text, cancel_style),
        Span::raw(" ".repeat(pad_right)),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}
