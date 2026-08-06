//! View — skill hub overlay (`/skill`).
//!
//! Renders as an overlay anchored above the composer (same pattern as `/bash`,
//! `/todo`, `/model`). Layout:
//!
//! 1. Header: ` skills ` on a `Borders::BOTTOM` rule (dim).
//! 2. Search line: the live `query` with a block cursor.
//! 3. Chip row: `[X]all  [ ]active` filter toggles.
//! 4. Filtered list: name + `[active]` badge + description.
//! 5. Footer: full-width inverse hint bar.

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::{SkillCmdState, SkillFilterChip};
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;

/// Width the name column is padded to (so descriptions align in a column).
const NAME_W: usize = 24;

/// Render the skill hub overlay.
///
/// `input_rect` is the composer rect (from `chat::layout_chunks()[4]`).
/// `transcript_rect` is the transcript rect (from `chat::layout_chunks()[1]`).
pub fn render_overlay(
    frame: &mut Frame,
    input_rect: Rect,
    transcript_rect: Rect,
    st: &SkillCmdState,
    rest: &AppStateRest,
    palette: &Palette,
) {
    // Compute the overlay rect: anchored above the composer, extending upward
    // into the transcript area. Same approach as bash/todo overlays.
    let overlay_height = 14u16.min(transcript_rect.height);
    let overlay_y = input_rect.y.saturating_sub(overlay_height);
    let overlay_rect = Rect {
        x: input_rect.x,
        y: overlay_y,
        width: input_rect.width,
        height: overlay_height,
    };

    // Clear the overlay background
    crate::view::clear_and_fill(frame, overlay_rect, palette.bg);

    // Inner vertical zones: header | search | chips | list | footer
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2), // header text + BOTTOM border
            Constraint::Length(1), // search line
            Constraint::Length(1), // chip row
            Constraint::Min(0),    // filtered list
            Constraint::Length(1), // footer hint
        ])
        .split(overlay_rect);

    // --- Header ---
    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled("skills", Style::default().fg(palette.dim))),
        header_inner.inner(Margin {
            horizontal: 2,
            vertical: 0,
        }),
    );

    // --- Search line (live query + block cursor) ---
    let search_inner = outer[1].inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    let search_line = Line::from(vec![
        Span::styled("› ", Style::default().fg(palette.dim)),
        Span::styled(st.query.as_str(), Style::default().fg(palette.fg)),
        Span::styled("█", Style::default().fg(palette.accent)),
    ]);
    frame.render_widget(Paragraph::new(search_line), search_inner);

    // --- Chip row ---
    let chip_inner = outer[2].inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    let all_style = match st.chip {
        SkillFilterChip::All => Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD),
        SkillFilterChip::Active => Style::default().fg(palette.dim),
    };
    let active_style = match st.chip {
        SkillFilterChip::Active => Style::default()
            .fg(palette.sel_fg)
            .bg(palette.sel_bg)
            .add_modifier(Modifier::BOLD),
        SkillFilterChip::All => Style::default().fg(palette.dim),
    };
    let chip_line = Line::from(vec![
        Span::styled("[X]", all_style),
        Span::styled("all ", all_style),
        Span::styled("[ ]", active_style),
        Span::styled("active", active_style),
    ]);
    frame.render_widget(Paragraph::new(chip_line), chip_inner);

    // --- Filtered list (windowed) ---
    let list_inner = outer[3].inner(Margin {
        horizontal: 2,
        vertical: 0,
    });
    let max_vis = list_inner.height as usize;

    if st.filtered_idx.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no matches", Style::default().fg(palette.dim))),
            list_inner,
        );
    } else if max_vis > 0 {
        let sel = st.selected.min(st.filtered_idx.len() - 1);
        let (start, end) = crate::view::scroll::scroll_window(
            &rest.skill_offset,
            sel,
            st.filtered_idx.len(),
            max_vis,
        );

        let rows: Vec<Line> = st.filtered_idx[start..end]
            .iter()
            .enumerate()
            .map(|(vi, &ai)| {
                let i = start + vi;
                let entry = &st.all[ai];
                let name_col = format!(" {:<NAME_W$}", entry.name);
                let badge = if entry.is_active {
                    " [active] "
                } else {
                    "          "
                };
                if i == sel {
                    let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                    Line::from(vec![
                        Span::styled(name_col, hl),
                        Span::styled(badge, hl),
                        Span::styled(&entry.description, hl),
                    ])
                } else {
                    let name_style = if entry.is_active {
                        Style::default().fg(palette.accent)
                    } else {
                        Style::default().fg(palette.fg)
                    };
                    let badge_style = if entry.is_active {
                        Style::default().fg(palette.success)
                    } else {
                        Style::default().fg(palette.dim)
                    };
                    Line::from(vec![
                        Span::styled(name_col, name_style),
                        Span::styled(badge, badge_style),
                        Span::styled(entry.description.clone(), Style::default().fg(palette.dim)),
                    ])
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(rows), list_inner);
    }

    // --- Footer: full-width inverse hint bar ---
    let footer_rect = outer[4];
    if footer_rect.width > 0 {
        let hint = "enter toggle · ←/→ filter · esc close";
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
