use ratatui::{
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::app::mode::SettingsState;
use crate::app::state::AppStateRest;
use crate::view::theme::Palette;

/// Render the coolors-style vertical palette list for the Appearance page.
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
pub(crate) fn draw_appearance(
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
        let roles: [ratatui::style::Color; 9] = [
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
