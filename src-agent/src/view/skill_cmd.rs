//! View — skill hub overlay (`/skill`).
//!
//! Composer-anchored overlay in the bash/todo/model family:
//!
//! ```text
//! avail = input.y - transcript.y
//! h     = desired.min(avail.max(3))
//! y     = input.y - h
//! ```
//!
//! Layout inside `Block::bordered` + dim title ` skills `:
//!
//! 1. Search line: live `query` + block cursor.
//! 2. Chip row: working radio `[x]all | [ ]active | [ ]inactive` — the `[x]`
//!    moves with ←/→ / Tab (exactly one checked).
//! 3. Filtered list: `[x]`/`[ ]` load mark + name + truncated description
//!    (one logical row = one scroll row; no wrap).
//! 4. Dim single-line footer hint (not inverse bar).

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Paragraph},
    Frame,
};

use crate::app::mode::{SkillCmdState, SkillFilterChip};
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;

/// Width the name column is padded to (so descriptions align).
const NAME_W: usize = 22;

/// Cap visible list rows before height clamp (matches model/bash family).
const LIST_CAP: u16 = 10;

/// Truncate `s` to at most `max` chars, appending `…` if cut.
fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        s.to_string()
    } else {
        let cut = max.saturating_sub(1);
        chars[..cut].iter().collect::<String>() + "…"
    }
}

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
    // Content budget: search + chips + list (capped) + footer + 2 border rows.
    let list_rows = (st.filtered_idx.len().max(1) as u16).min(LIST_CAP);
    let desired = list_rows + 2 + 1 + 1 + 1; // border + search + chips + list + footer
    let avail = input_rect.y.saturating_sub(transcript_rect.y);
    let h = desired.min(avail.max(3));
    let y = input_rect.y.saturating_sub(h);
    let overlay_rect = Rect {
        x: input_rect.x,
        y,
        width: input_rect.width,
        height: h,
    };

    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(" skills ", Style::default().fg(palette.dim)));
    let inner = block.inner(overlay_rect);
    crate::view::clear_and_fill(frame, overlay_rect, palette.bg);
    frame.render_widget(block, overlay_rect);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Inner vertical zones: search | chips | list | footer
    let zones = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // search line
            Constraint::Length(1), // chip row
            Constraint::Min(0),    // filtered list
            Constraint::Length(1), // footer hint
        ])
        .split(inner);

    // --- Search line (live query + block cursor) ---
    let search_inner = zones[0].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let search_line = Line::from(vec![
        Span::styled("› ", Style::default().fg(palette.dim)),
        Span::styled(st.query.as_str(), Style::default().fg(palette.fg)),
        Span::styled("█", Style::default().fg(palette.accent)),
    ]);
    frame.render_widget(Paragraph::new(search_line), search_inner);

    // --- Chip row: working radio — [x] moves with ←/→ across three filters ---
    let chip_inner = zones[1].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let mk_chip = |chip: SkillFilterChip, label: &str| -> (String, Style) {
        let on = st.chip == chip;
        let mark = if on { "[x]" } else { "[ ]" };
        let style = if on {
            Style::default()
                .fg(palette.sel_fg)
                .bg(palette.sel_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.dim)
        };
        (format!("{mark}{label}"), style)
    };
    let (all_txt, all_style) = mk_chip(SkillFilterChip::All, "all");
    let (active_txt, active_style) = mk_chip(SkillFilterChip::Active, "active");
    let (inactive_txt, inactive_style) = mk_chip(SkillFilterChip::Inactive, "inactive");
    let chip_line = Line::from(vec![
        Span::styled(all_txt, all_style),
        Span::styled(" | ", Style::default().fg(palette.dim)),
        Span::styled(active_txt, active_style),
        Span::styled(" | ", Style::default().fg(palette.dim)),
        Span::styled(inactive_txt, inactive_style),
    ]);
    frame.render_widget(Paragraph::new(chip_line), chip_inner);

    // --- Filtered list (windowed, one row each, truncated) ---
    let list_inner = zones[2].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let max_vis = list_inner.height as usize;
    let row_w = list_inner.width as usize;

    if st.filtered_idx.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("no matches", Style::default().fg(palette.dim))),
            list_inner,
        );
    } else if max_vis > 0 && row_w > 0 {
        let sel = st.selected.min(st.filtered_idx.len() - 1);
        let (start, end) = crate::view::scroll::scroll_window(
            &rest.skill_offset,
            sel,
            st.filtered_idx.len(),
            max_vis,
        );

        // "{mark} {name:<pad} {desc…}" — mark is load state, inverse is cursor.
        let mark_w = 3; // "[x]" / "[ ]"
        let name_pad = NAME_W.min(row_w.saturating_sub(mark_w + 2));
        let desc_budget = row_w.saturating_sub(mark_w + 1 + name_pad + 1);

        let rows: Vec<Line> = st.filtered_idx[start..end]
            .iter()
            .enumerate()
            .map(|(vi, &ai)| {
                let i = start + vi;
                let entry = &st.all[ai];
                let mark = if entry.is_active { "[x]" } else { "[ ]" };
                let name_col = format!("{:<w$}", truncate(&entry.name, name_pad), w = name_pad);
                let desc = if desc_budget > 0 {
                    truncate(&entry.description, desc_budget)
                } else {
                    String::new()
                };
                let text = if desc.is_empty() {
                    format!("{mark} {name_col}")
                } else {
                    format!("{mark} {name_col} {desc}")
                };
                let text = truncate(&text, row_w);
                let padded = format!("{:<width$}", text, width = row_w);

                if i == sel {
                    let hl = Style::default().fg(palette.sel_fg).bg(palette.sel_bg);
                    Line::from(Span::styled(padded, hl))
                } else {
                    let style = if entry.is_active {
                        Style::default().fg(palette.accent)
                    } else {
                        Style::default().fg(palette.fg)
                    };
                    Line::from(Span::styled(padded, style))
                }
            })
            .collect();

        frame.render_widget(Paragraph::new(rows), list_inner);
    }

    // --- Footer: dim single-line hint inside border ---
    let footer_inner = zones[3].inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    if footer_inner.width > 0 {
        let hint = "enter/space toggle · ←→/tab filter · esc";
        frame.render_widget(
            Paragraph::new(Span::styled(
                truncate(hint, footer_inner.width as usize),
                Style::default().fg(palette.dim),
            )),
            footer_inner,
        );
    }
}
