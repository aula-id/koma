use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::mode::{SettingField, SettingsState, SETTING_CATEGORIES};
use crate::model::app_config::ThemeMode;
use crate::view::theme::{resolve_accent, Palette};

use super::super::utils::truncate;

/// Render the field-row list for the General page (Session category, index 1
/// in SETTING_CATEGORIES). Shows each field as a `label   value` row with
/// selection highlighting, text editing, boolean toggles, and path-list
/// entries. No scroll-windowing yet — all fields render and clip naturally.
pub(crate) fn draw_general(
    frame: &mut Frame,
    st: &SettingsState,
    palette: &Palette,
    dark: bool,
    area: Rect,
) {
    let cat_fields = SETTING_CATEGORIES[1].fields;
    let detail_w = area.width as usize;
    let value_w = detail_w.saturating_sub(16);

    let mut detail_lines: Vec<Line> = Vec::new();
    for (i, &f) in cat_fields.iter().enumerate() {
        let is_selected = i == st.field;

        // Marker
        let marker = Span::styled(
            if is_selected { "› " } else { "  " },
            Style::default().fg(palette.accent),
        );

        // Label: left-padded to 14 cols.
        let label_text = format!("{:<14}", f.label());
        let label_color = if is_selected { palette.accent } else { palette.dim };
        let label_span = Span::styled(label_text, Style::default().fg(label_color));

        // PATH LISTS (Workdir / Allowed dirs): a label row, then one
        // line-wrapped row per entry.
        if SettingsState::is_path_list(f) {
            let managing = st.list_editing && is_selected;
            let label_suffix: Vec<Span> = if is_selected && !managing {
                vec![Span::styled("list", Style::default().fg(palette.dim))]
            } else {
                Vec::new()
            };
            let mut header = vec![marker, label_span];
            header.extend(label_suffix);
            detail_lines.push(Line::from(header));

            let entries = st.path_list(f).cloned().unwrap_or_default();
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
                let tint = resolve_accent(&st.accent, dark);
                vec![Span::styled(st.accent.as_str(), Style::default().fg(tint))]
            }
            SettingField::AwarenessEnabled => {
                let v = if st.awareness_enabled { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::ClassifierEnabled => {
                let v = if st.classifier_enabled { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::ShortSendEnabled => {
                let v = if st.short_send_enabled { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::SlidingCache => {
                let v = if st.sliding_cache { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::BashSaving => {
                let v = if st.bash_saving { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::CodingAutosave => {
                let v = if st.coding_autosave { "on" } else { "off" };
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::InternetMode => {
                let v = st.internet_mode.as_str();
                vec![Span::styled(v, Style::default().fg(palette.accent))]
            }
            SettingField::AwarenessSource => {
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
                vec![Span::styled("(inherited)", Style::default().fg(palette.dim))]
            }
            _ => {
                let raw: &str = match f {
                    SettingField::ApiKey   => st.api_key.as_str(),
                    SettingField::Model    => st.model.as_str(),
                    SettingField::Provider => {
                        if st.provider.is_empty() {
                            ""
                        } else {
                            st.provider.as_str()
                        }
                    }
                    SettingField::Name    => st.name.as_str(),
                    SettingField::AwarenessModel    => st.awareness_model.as_str(),
                    SettingField::AwarenessProvider => st.awareness_provider.as_str(),
                    SettingField::ClassifierModel    => st.classifier_model.as_str(),
                    SettingField::ClassifierProvider => st.classifier_provider.as_str(),
                    _ => "",
                };
                let editing_here = st.editing && is_selected;
                let truncate_w = if editing_here {
                    value_w.saturating_sub(1)
                } else {
                    value_w
                };
                if f == SettingField::Provider && raw.is_empty() && !editing_here {
                    detail_lines.push(Line::from(vec![
                        marker,
                        label_span,
                        Span::styled("default", Style::default().fg(palette.dim)),
                    ]));
                    continue;
                }
                let display_raw = if f == SettingField::ApiKey {
                    truncate(raw, truncate_w.min(40))
                } else {
                    truncate(raw, truncate_w)
                };
                let mut shown = display_raw;
                if editing_here {
                    shown.push('\u{2588}');
                }
                vec![Span::styled(shown, Style::default().fg(palette.fg))]
            }
        };

        let mut spans = vec![marker, label_span];
        spans.extend(value_spans);
        detail_lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(detail_lines), area);
}
