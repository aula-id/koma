//! View – in-app settings (Settings mode).
//!
//! The menu page renders as a compact overlay anchored above the input bar,
//! following the same pattern as bash/todo/commands overlays. Sub-pages
//! (Appearance, Providers, Models, etc.) render fullscreen (no chat underneath).
//! A breadcrumb header shows the current route; Esc always goes back one level.
//! Only transient pickers (FS directory, role checkbox, OAuth flow states) render
//! as sub-overlays within the current view.
//!
//! Border convention (strict, matches project rules):
//! - Header: `Borders::BOTTOM` only, with breadcrumb text.
//! - Footer: inverse full-width hint bar.
//! - No sidebar / dual-pane — every page fills the body area.

pub(crate) mod oauth;
mod pages;
mod pickers;
mod utils;

use crate::app::mode::settings::{OAuthFlowState, SettingsPage};
use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;
use crate::model::app_config::ThemeMode;
use crate::view::theme::Palette;
use pickers::draw_role_picker;
use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph},
    Frame,
};

/// Render the settings menu as a compact overlay anchored above the input bar,
/// following the same pattern as the slash-command palette.
pub fn render_menu_overlay(
    frame: &mut Frame,
    st: &SettingsState,
    palette: &Palette,
    input_chunk: Rect,
    transcript_chunk: Rect,
) {
    let rows: Vec<Line> = pages::menu::MENU_ITEMS
        .iter()
        .enumerate()
        .map(|(i, (num, label, _page))| {
            let is_selected = i == st.menu_sel;
            let style = if is_selected {
                Style::default()
                    .fg(palette.sel_fg)
                    .bg(palette.sel_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.fg)
            };
            let chip = Span::styled(
                format!("[{num}]"),
                if is_selected {
                    style
                } else {
                    Style::default().fg(palette.accent)
                },
            );
            let text = Span::styled(format!("  {label}"), style);
            Line::from(vec![Span::raw(" "), chip, text])
        })
        .collect();

    // Content-sized height (+2 for borders), clamped to available space.
    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = ((rows.len() as u16) + 2).min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let popup = Rect {
        x: input_chunk.x,
        y,
        width: input_chunk.width,
        height: h,
    };

    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(" settings ", Style::default().fg(palette.dim)))
        .padding(Padding::horizontal(1));
    let inner = block.inner(popup);
    crate::view::clear_and_fill(frame, popup, palette.bg);
    frame.render_widget(block, popup);
    frame.render_widget(Paragraph::new(rows), inner);
}

/// Render the settings content inside the given `area` rect.
pub fn draw(
    frame: &mut Frame,
    rest: &AppStateRest,
    st: &SettingsState,
    models_cache: &[crate::dto::openrouter::ModelInfo],
    cache_endpoint: Option<&str>,
    palette: &Palette,
    area: Rect,
) {
    let dark = st.theme == ThemeMode::Dark;

    // Outer vertical zones: header | body | footer.
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Min(0),    // body (page content)
            Constraint::Length(1), // footer key hints
        ])
        .split(area);

    // --- Breadcrumb header ---
    let breadcrumb = breadcrumb_text(st.page);
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(breadcrumb, Style::default().fg(palette.dim)))
            .style(Style::default()),
        header_inner.inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
    );

    // --- Body: page dispatch ---
    let body = outer[1];
    let body_inner = body.inner(Margin {
        horizontal: 2,
        vertical: 1,
    });

    match st.page {
        SettingsPage::Menu => {
            pages::draw_menu(frame, st.menu_sel, palette, body);
        }
        SettingsPage::Appearance => {
            pages::draw_appearance(
                frame,
                rest,
                st,
                palette,
                rest.config.palette.as_str(),
                body_inner,
            );
        }
        SettingsPage::General => {
            pages::draw_general(frame, st, palette, dark, body_inner);
        }
        SettingsPage::Providers => {
            pages::draw_providers_page(frame, st, palette, body_inner);
        }
        SettingsPage::ProviderForm => {
            if let Some(modal) = st.prov_modal.as_ref() {
                pages::draw_provider_form(frame, modal, palette, body_inner);
            }
        }
        SettingsPage::OAuth => {
            // The idle connection list is drawn as the page body. OAuth flow
            // overlays (picker, wait, paste, failed) are drawn AFTER the
            // body as floating overlays below.
            pages::draw_oauth_page(frame, st, palette, body_inner);
        }
        SettingsPage::Models => {
            pages::draw_models_page(frame, rest, st, palette, body_inner);
        }
        SettingsPage::ModelForm => {
            if let Some(modal) = st.model_modal.as_ref() {
                let omni = st.mm_provider_omnisearchable();
                let is_or = st.mm_provider_is_openrouter();
                let is_codex = st.mm_selected_is_codex();
                let is_static_overlay = st.mm_selected_is_static_overlay();
                let static_cache = if is_codex {
                    crate::service::oauth::registry::codex_static_catalogue()
                } else if is_static_overlay {
                    st.mm_static_overlay_catalogue()
                } else {
                    Vec::new()
                };
                let (cache, cache_matches): (&[crate::dto::openrouter::ModelInfo], bool) =
                    if is_codex || is_static_overlay {
                        (&static_cache, true)
                    } else {
                        let cm = st
                            .mm_provider_conn()
                            .map(|(ep, _)| cache_endpoint == Some(ep.as_str()))
                            .unwrap_or(false);
                        (models_cache, cm)
                    };
                let catalogue_failed = st
                    .mm_provider_conn()
                    .map(|(ep, _)| rest.models_cache_failed.as_deref() == Some(ep.as_str()))
                    .unwrap_or(false);
                pages::draw_model_form(
                    frame,
                    rest,
                    st,
                    modal,
                    omni,
                    is_or,
                    cache_matches,
                    catalogue_failed,
                    cache,
                    palette,
                    body_inner,
                );

                // Role checkbox picker overlay: drawn over the model form.
                if let Some(picker) = modal.role_picker.as_ref() {
                    draw_role_picker(frame, picker, palette, area);
                }
            }
        }
    }

    // --- OAuth flow overlays (float over the OAuth page body) ---
    if st.page == SettingsPage::OAuth {
        match &st.oauth_flow {
            OAuthFlowState::Pick(cursor) => {
                oauth::draw_picker(frame, *cursor, palette, body);
            }
            OAuthFlowState::CodexWait {
                provider,
                url,
                copied,
                ..
            } => {
                let title = format!("{} login", provider.label());
                oauth::draw_message(frame, palette, body, &title, Some(url), *copied);
            }
            OAuthFlowState::KiloWait {
                provider,
                verification_url,
                copied,
                ..
            } => {
                let title = format!("{} login", provider.label());
                oauth::draw_message(
                    frame,
                    palette,
                    body,
                    &title,
                    Some(verification_url),
                    *copied,
                );
            }
            OAuthFlowState::CodexPaste { input, .. } => {
                oauth::draw_paste(frame, input, palette, body);
            }
            OAuthFlowState::Failed(msg) => {
                oauth::draw_failed(frame, msg, palette, body);
            }
            OAuthFlowState::Starting { provider } => {
                let title = format!("starting {} login\u{2026}", provider.label());
                oauth::draw_message(frame, palette, body, &title, None, false);
            }
            _ => {}
        }
    }

    // --- FS directory picker overlay (floats over any page) ---
    if let Some(picker) = st.picker.as_ref() {
        const MAX_VIS: usize = crate::app::mode::PICKER_MAX;

        let mut rows: Vec<Line> = Vec::new();
        rows.push(Line::from(vec![
            Span::styled("@ ", Style::default().fg(palette.accent)),
            Span::styled(picker.query.as_str(), Style::default().fg(palette.fg)),
            Span::styled("\u{2588}", Style::default().fg(palette.accent)),
        ]));

        if picker.matches.is_empty() {
            rows.push(Line::from(Span::styled(
                "  (no matching directories)",
                Style::default().fg(palette.dim),
            )));
        } else {
            let sel = picker.sel.min(picker.matches.len().saturating_sub(1));
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

        let title = if picker.matches.len() > MAX_VIS {
            format!(
                " pick directory {}/{} ",
                picker.sel + 1,
                picker.matches.len()
            )
        } else {
            " pick directory ".to_string()
        };

        let h = ((rows.len() as u16) + 2).min(body.height.max(3));
        let w = body.width.saturating_sub(4).max(10);
        let x = body.x + (body.width.saturating_sub(w)) / 2;
        let y = body.y + (body.height.saturating_sub(h)) / 2;
        let popup = Rect {
            x,
            y,
            width: w,
            height: h,
        };

        let block = Block::bordered()
            .border_style(Style::default().fg(palette.dim))
            .title(Span::styled(title, Style::default().fg(palette.dim)))
            .padding(Padding::horizontal(1));
        let inner = block.inner(popup);
        crate::view::clear_and_fill(frame, popup, palette.bg);
        frame.render_widget(block, popup);
        frame.render_widget(Paragraph::new(rows), inner);
    }

    // --- Footer: context-sensitive hint bar ---
    let footer_rect = outer[2];
    if footer_rect.width > 0 {
        let hint = footer_hint(st);
        let bar_style = Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD);
        let padded = format!(
            " {:<width$}",
            hint,
            width = footer_rect.width.saturating_sub(1) as usize
        );
        frame.render_widget(
            Paragraph::new(Line::from(Span::raw(padded))).style(bar_style),
            footer_rect,
        );
    }
}

/// Build the breadcrumb string for the header.
fn breadcrumb_text(page: SettingsPage) -> String {
    match page {
        SettingsPage::Menu => "settings".to_string(),
        SettingsPage::Appearance => "settings > Appearance".to_string(),
        SettingsPage::General => "settings > General".to_string(),
        SettingsPage::Providers => "settings > Providers".to_string(),
        SettingsPage::ProviderForm => "settings > Providers > Add".to_string(),
        SettingsPage::OAuth => "settings > OAuth".to_string(),
        SettingsPage::Models => "settings > Models".to_string(),
        SettingsPage::ModelForm => "settings > Models > Add".to_string(),
    }
}

/// Build the context-sensitive footer hint for the current state.
fn footer_hint(st: &SettingsState) -> String {
    use crate::app::mode::settings::ModelField;

    // Deepest-first: overlays own the hint.
    if st
        .model_modal
        .as_ref()
        .map(|m| m.role_picker.is_some())
        .unwrap_or(false)
    {
        return "↑↓ role · space toggle · enter ok · esc cancel".to_string();
    }
    if let Some(modal) = st.model_modal.as_ref() {
        let cur = st.mm_current_field();
        let omni = st.mm_provider_omnisearchable();
        let search = cur == Some(ModelField::Model) && omni && !modal.query.is_empty();
        if search {
            return "↑↓ result · enter pick · tab next · esc cancel".to_string();
        }
        if cur == Some(ModelField::Route) {
            return "↑↓ provider/move · enter pin + next · esc cancel".to_string();
        }
        if cur == Some(ModelField::Role) {
            return "enter roles · esc cancel".to_string();
        }
        return "↑↓ field · ←→ provider · enter select · esc cancel".to_string();
    }
    if st.prov_modal.is_some() {
        return "↑↓ field · ←→ move/type · enter select · esc cancel".to_string();
    }
    if st.picker.is_some() {
        return "type path · @rel or /abs · ↑/↓ select · Tab descend · Enter pick · Esc cancel"
            .to_string();
    }
    if st.list_editing {
        return "↑/↓ entry · + add · - remove · Enter edit · Esc done".to_string();
    }
    if st.editing {
        return "type to edit · Enter/Esc done".to_string();
    }

    match st.page {
        SettingsPage::Menu => "1-5 select · esc save & close".to_string(),
        SettingsPage::Appearance => "↑↓ palette · enter apply · esc back".to_string(),
        SettingsPage::General => "↑↓ field · enter edit/toggle · esc back".to_string(),
        SettingsPage::Providers => {
            if let Some(msg) = st.prov_msg.as_deref() {
                return msg.to_string();
            }
            if st.prov_delete_armed {
                "ctrl+x again to CONFIRM delete · any key cancels".to_string()
            } else {
                "↑↓ select · a add · ctrl+x delete · esc back".to_string()
            }
        }
        SettingsPage::OAuth => match &st.oauth_flow {
            OAuthFlowState::Idle => {
                if st.oauth_armed.is_some() {
                    "ctrl+x again to CONFIRM delete · any key cancels".to_string()
                } else {
                    "↑↓ select · enter connect · ctrl+x delete · esc back".to_string()
                }
            }
            OAuthFlowState::Pick(_) => "↑↓ select · enter choose · esc back".to_string(),
            OAuthFlowState::CodexPaste { .. } => "type token · enter save · esc back".to_string(),
            OAuthFlowState::Failed(_) => "enter/esc dismiss".to_string(),
            OAuthFlowState::CodexWait { .. } | OAuthFlowState::KiloWait { .. } => {
                "c copy url · o open browser · esc cancel".to_string()
            }
            _ => "esc cancel".to_string(),
        },
        SettingsPage::Models => {
            if st.model_delete_armed {
                "ctrl+x again to CONFIRM delete · any key cancels".to_string()
            } else {
                "↑↓ line · ←→ item · space select · enter open · a add · esc back".to_string()
            }
        }
        SettingsPage::ProviderForm => "↑↓ field · enter advance · esc back".to_string(),
        SettingsPage::ModelForm => "↑↓ field · enter select · esc back".to_string(),
    }
}
