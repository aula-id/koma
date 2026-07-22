use super::super::utils::{price_per_million, truncate};
use crate::app::mode::filter_models;
use crate::app::mode::settings::{ModelField, ModelModal, ModelRole};
use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;
use crate::dto::openrouter::ModelInfo;
use crate::view::theme::Palette;
use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

/// Render the add/edit-model form as a full page.
///
/// - `omni` = `st.mm_provider_omnisearchable()` — the Model field is the live
///   omnisearch (any provider with a non-empty endpoint), not a plain text box.
/// - `is_or` = `st.mm_provider_is_openrouter()` — gates the Route upstream-pin
///   section (OpenRouter-only).
/// - `cache_matches` — whether `cache` was fetched for THIS provider's endpoint.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_model_form(
    frame: &mut Frame,
    rest: &AppStateRest,
    st: &SettingsState,
    modal: &ModelModal,
    omni: bool,
    is_or: bool,
    cache_matches: bool,
    cache: &[ModelInfo],
    palette: &Palette,
    area: Rect,
) {
    let inner = area;
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let label_w = 10usize;
    let val_w = (inner.width as usize).saturating_sub(label_w + 1).max(4);
    let mut lines: Vec<Line> = Vec::new();

    let fields = st.model_modal_fields();
    let focused = |f: ModelField| fields.get(modal.field).copied() == Some(f);

    // Row: Name.
    {
        let active = focused(ModelField::Name);
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(
            format!("{:<width$}", "Name", width = label_w),
            Style::default().fg(lc),
        );
        let mut val = truncate(&modal.name, val_w.saturating_sub(1));
        if active {
            val.push('\u{2588}');
        }
        let vc = if active { palette.fg } else { palette.dim };
        lines.push(Line::from(vec![
            label,
            Span::styled(val, Style::default().fg(vc)),
        ]));
    }

    // Row: Provider toggle.
    {
        let active = focused(ModelField::Provider);
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(
            format!("{:<width$}", "Provider", width = label_w),
            Style::default().fg(lc),
        );
        let prov_name = st.mm_provider_label();
        let toggle_text = match prov_name.as_deref() {
            Some(n) => format!("\u{2039} {} \u{203a}", n),
            None => "\u{2039} (no providers) \u{203a}".to_string(),
        };
        let tc = if active { palette.accent } else { palette.dim };
        lines.push(Line::from(vec![
            label,
            Span::styled(toggle_text, Style::default().fg(tc)),
        ]));
    }

    // Row(s): Model.
    {
        let active = focused(ModelField::Model);
        let lc = if active { palette.accent } else { palette.dim };
        let label = Span::styled(
            format!("{:<width$}", "Model", width = label_w),
            Style::default().fg(lc),
        );

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
                    if active {
                        ph.push('\u{2588}');
                    }
                    Span::styled(ph, Style::default().fg(palette.dim))
                } else {
                    let mut q = truncate(&modal.query, val_w.saturating_sub(1));
                    if active {
                        q.push('\u{2588}');
                    }
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
                    let (start, end) = crate::view::scroll::scroll_window(
                        &rest.model_modal_results_offset,
                        sel,
                        results.len(),
                        MAX_VIS,
                    );
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
                                format!("{text:<row_w$}"),
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

            // --- 5. Route field: label row + selectable options list (OpenRouter only) ---
            if is_or && modal.query.is_empty() {
                let row_w = inner.width as usize;
                let route_active = focused(ModelField::Route);

                if !modal.model_id.is_empty() {
                    let lc = if route_active {
                        palette.accent
                    } else {
                        palette.dim
                    };
                    let rl = Span::styled(
                        format!("{:<width$}", "Route", width = label_w),
                        Style::default().fg(lc),
                    );
                    let choice = match modal.route.as_deref() {
                        Some(name) if !name.is_empty() => name.to_string(),
                        _ => "Auto (OpenRouter routes)".to_string(),
                    };
                    let vc = if route_active {
                        palette.fg
                    } else {
                        palette.dim
                    };
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
                        let option_count = 1 + eps.len();
                        let sel = modal.route_sel.min(option_count - 1);
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

                        const MAX_EP: usize = 8;
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
                            opt_labels.push(format!("{name:<14} {price}  {uptime}"));
                        }

                        const VIS: usize = MAX_EP + 1;
                        let (start, end) = if route_active {
                            crate::view::scroll::scroll_window(
                                &rest.model_modal_route_offset,
                                sel,
                                option_count,
                                VIS,
                            )
                        } else {
                            rest.model_modal_route_offset.set(0);
                            (0, VIS.min(option_count))
                        };
                        for (i, label) in opt_labels.iter().enumerate().take(end).skip(start) {
                            let marker = if i == pinned { "\u{2023} " } else { "  " };
                            let text = truncate(&format!("{marker}{label}"), row_w);
                            let style = if route_active && i == sel {
                                Style::default().fg(palette.sel_fg).bg(palette.sel_bg)
                            } else if i == pinned {
                                Style::default().fg(palette.accent)
                            } else {
                                Style::default().fg(palette.fg)
                            };
                            lines.push(Line::from(Span::styled(format!("{text:<row_w$}"), style)));
                        }
                        if end < option_count {
                            lines.push(Line::from(Span::styled(
                                format!("+{} more", option_count - end),
                                Style::default().fg(palette.dim),
                            )));
                        }
                    }
                } else if !modal.model_id.is_empty() {
                    lines.push(Line::from(Span::styled(
                        "loading routes\u{2026}",
                        Style::default().fg(palette.dim),
                    )));
                } else {
                    lines.push(Line::from(Span::styled(
                        "pick a model to see providers",
                        Style::default().fg(palette.dim),
                    )));
                }
            }
        } else {
            // Non-OpenRouter: plain editable model id with a bottom rule.
            let mut val = truncate(&modal.model_id, val_w.saturating_sub(1));
            if active {
                val.push('\u{2588}');
            }
            let vc = if active { palette.fg } else { palette.dim };
            lines.push(Line::from(vec![
                label,
                Span::styled(val, Style::default().fg(vc)),
            ]));

            let rule_w = val_w.max(1);
            let rule_str = "\u{2500}".repeat(rule_w);
            lines.push(Line::from(vec![
                Span::raw(" ".repeat(label_w)),
                Span::styled(rule_str, Style::default().fg(palette.dim)),
            ]));
        }
    }

    // Row: Role readout (edit mode only).
    let active = focused(ModelField::Role);
    let lc = if active { palette.accent } else { palette.dim };
    let label = Span::styled(
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

    // Blank line before the buttons.
    lines.push(Line::from(""));

    // Button row: `[ Save ]  [ Cancel ]` centered.
    let save_text = "[ Save ]";
    let cancel_text = "[ Cancel ]";
    let gap = "  ";
    let group_len = save_text.len() + gap.len() + cancel_text.len();
    let inner_w = inner.width as usize;
    let pad_left = inner_w.saturating_sub(group_len) / 2;
    let pad_right = inner_w.saturating_sub(group_len).saturating_sub(pad_left);
    let save_style = if focused(ModelField::Save) {
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
        Span::styled(cancel_text, cancel_style),
        Span::raw(" ".repeat(pad_right)),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}
