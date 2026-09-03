//! Ctrl+P attachments overlay view (list + nested paste editor).

use ratatui::{
    layout::{Constraint, Direction, Layout, Margin},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};

use crate::app::mode::AttachmentsState;
use crate::app::state::AppStateRest;
use crate::dto::chat::AttachmentKind;
use crate::view::theme::Palette;

/// Render list overlay above the chat input (Todo-style). When a nested
/// editor is open, draw a full-frame paste editor instead.
pub fn render_attachments_overlay(
    frame: &mut Frame,
    input_chunk: ratatui::layout::Rect,
    transcript_chunk: ratatui::layout::Rect,
    st: &AttachmentsState,
    rest: &AppStateRest,
    palette: &Palette,
) {
    if let Some((n, ref ed)) = st.editor.as_ref() {
        draw_paste_editor(frame, ed, *n, palette);
        return;
    }

    let avail = input_chunk.y.saturating_sub(transcript_chunk.y);
    let h = 12u16.min(avail.max(3));
    let y = input_chunk.y.saturating_sub(h);
    let rect = ratatui::layout::Rect {
        x: input_chunk.x,
        y,
        width: input_chunk.width,
        height: h,
    };

    let title = format!(" attachments ({}) ", st.items.len());
    let block = Block::bordered()
        .border_style(Style::default().fg(palette.dim))
        .title(Span::styled(title, Style::default().fg(palette.dim)));
    let inner = block.inner(rect);
    crate::view::clear_and_fill(frame, rect, palette.bg);
    frame.render_widget(block, rect);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if st.items.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "(no staged attachments — paste text ≥150 chars / multi-line, or attach an image)",
                Style::default().fg(palette.dim),
            )),
            inner.inner(Margin {
                horizontal: 1,
                vertical: 0,
            }),
        );
        return;
    }

    const LIST_W: u16 = 28;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(LIST_W), Constraint::Min(0)])
        .split(inner);

    let list_block = Block::new()
        .borders(Borders::RIGHT)
        .border_style(Style::default().fg(palette.dim));
    let list_inner = list_block.inner(cols[0]);
    frame.render_widget(list_block, cols[0]);

    let sel = st.selected.min(st.items.len().saturating_sub(1));
    let list_w = list_inner.width as usize;
    let list_lines: Vec<Line> = st
        .items
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let kind = AttachmentsState::kind_label(a.kind);
            let label = format!(" {kind} #{n} ", n = a.marker_n);
            let mut s = label;
            if s.chars().count() > list_w {
                s = s.chars().take(list_w.saturating_sub(1)).collect::<String>() + "…";
            }
            let style = if i == sel {
                Style::default()
                    .fg(palette.sel_fg)
                    .bg(palette.sel_bg)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(palette.fg)
            };
            Line::from(Span::styled(s, style))
        })
        .collect();
    frame.render_widget(Paragraph::new(list_lines), list_inner);

    // Detail pane.
    let att = &st.items[sel];
    let mut detail: Vec<Line> = Vec::new();
    detail.push(Line::from(Span::styled(
        format!(
            "{} #{}",
            AttachmentsState::kind_label(att.kind),
            att.marker_n
        ),
        Style::default()
            .fg(palette.accent)
            .add_modifier(Modifier::BOLD),
    )));
    detail.push(Line::from(Span::styled(
        att.rel_path.clone(),
        Style::default().fg(palette.dim),
    )));
    detail.push(Line::from(Span::styled(
        att.mime.clone(),
        Style::default().fg(palette.dim),
    )));
    if att.kind == AttachmentKind::PastedText {
        if let Some(sess) = rest.fg().session.as_ref() {
            let path = sess.path.join(&att.rel_path);
            if let Ok(body) = std::fs::read_to_string(&path) {
                let preview: String = body.lines().take(6).collect::<Vec<_>>().join("\n");
                detail.push(Line::from(""));
                for line in preview.lines() {
                    detail.push(Line::from(Span::styled(
                        line.to_string(),
                        Style::default().fg(palette.fg),
                    )));
                }
            }
        }
        detail.push(Line::from(""));
        detail.push(Line::from(Span::styled(
            "Enter edit · d remove · Esc close",
            Style::default().fg(palette.dim),
        )));
    } else {
        detail.push(Line::from(""));
        detail.push(Line::from(Span::styled(
            "d remove · Esc close",
            Style::default().fg(palette.dim),
        )));
    }
    frame.render_widget(
        Paragraph::new(detail),
        cols[1].inner(Margin {
            horizontal: 1,
            vertical: 0,
        }),
    );
}

fn draw_paste_editor(
    frame: &mut Frame,
    ed: &crate::app::mode::editor::TextEditorState,
    n: usize,
    palette: &Palette,
) {
    // Reuse the agents field-editor layout without depending on AgentsState.
    let area = frame.area();
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(area);

    let header_block = Block::new()
        .borders(Borders::BOTTOM)
        .border_style(Style::default().fg(palette.dim));
    let header_inner = header_block.inner(outer[0]);
    frame.render_widget(header_block, outer[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!("edit pasted text #{n}"),
            Style::default().fg(palette.dim),
        )),
        header_inner.inner(Margin {
            horizontal: 1,
            vertical: 0,
        }),
    );

    // Publish wrap width for visual Up/Down.
    let body_w = outer[1].width.saturating_sub(6) as usize;
    let wrap_w = body_w.max(1);
    ed.wrap_w.set(wrap_w);

    let mut lines: Vec<Line> = Vec::new();
    for (li, logical) in ed.lines.iter().enumerate() {
        let chars: Vec<char> = logical.chars().collect();
        let segs = crate::app::mode::editor::wrap_segments(&chars, wrap_w);
        for (si, (start, end)) in segs.iter().enumerate() {
            let text: String = chars[*start..*end].iter().collect();
            let gutter = if si == 0 {
                format!("{:>4} ", li + 1)
            } else {
                "     ".to_string()
            };
            lines.push(Line::from(vec![
                Span::styled(gutter, Style::default().fg(palette.dim)),
                Span::styled(text, Style::default().fg(palette.fg)),
            ]));
        }
    }
    // Scroll: keep cursor visual row on screen.
    let mut cursor_visual = 0usize;
    {
        let mut v = 0usize;
        for (li, logical) in ed.lines.iter().enumerate() {
            let chars: Vec<char> = logical.chars().collect();
            let segs = crate::app::mode::editor::wrap_segments(&chars, wrap_w);
            if li == ed.row {
                for (si, (start, end)) in segs.iter().enumerate() {
                    if ed.col >= *start && (ed.col < *end || (ed.col == *end && si + 1 == segs.len()))
                    {
                        cursor_visual = v + si;
                        break;
                    }
                }
                break;
            }
            v += segs.len();
        }
    }
    let body_h = outer[1].height as usize;
    let mut scroll = ed.scroll;
    if cursor_visual < scroll {
        scroll = cursor_visual;
    } else if body_h > 0 && cursor_visual >= scroll + body_h {
        scroll = cursor_visual + 1 - body_h;
    }
    // Can't mutate ed.scroll through & — local only for render window.
    let visible: Vec<Line> = lines.into_iter().skip(scroll).take(body_h.max(1)).collect();
    frame.render_widget(Paragraph::new(visible), outer[1]);

    frame.render_widget(
        Paragraph::new(Span::styled(
            " ↑↓←→ move · Enter newline · Esc save & back ",
            Style::default().fg(palette.dim),
        )),
        outer[2],
    );

    // Place terminal cursor.
    let mut v = 0usize;
    let mut cx = outer[1].x + 5;
    let mut cy = outer[1].y;
    for (li, logical) in ed.lines.iter().enumerate() {
        let chars: Vec<char> = logical.chars().collect();
        let segs = crate::app::mode::editor::wrap_segments(&chars, wrap_w);
        if li == ed.row {
            for (si, (start, end)) in segs.iter().enumerate() {
                let row_v = v + si;
                if row_v < scroll {
                    continue;
                }
                if ed.col >= *start && (ed.col <= *end) {
                    let col_off = ed.col.saturating_sub(*start);
                    cx = outer[1].x + 5 + col_off as u16;
                    cy = outer[1].y + (row_v - scroll) as u16;
                    break;
                }
            }
            break;
        }
        v += segs.len();
    }
    if cy < outer[1].y + outer[1].height {
        frame.set_cursor_position(ratatui::layout::Position { x: cx, y: cy });
    }
}
