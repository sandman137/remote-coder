//! Rendering: `GridSnapshot` → ratatui widgets. Pure functions of `App`.

use engine::{Cell, CellAttrs, Color as GridColor};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph};
use ratatui::Frame;

use super::{App, View};

pub fn draw(frame: &mut Frame, app: &mut App) {
    match app.view {
        View::List => draw_list(frame, app),
        View::Pane => draw_pane(frame, app),
    }
}

fn draw_list(frame: &mut Frame, app: &mut App) {
    let [body, help] =
        Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(frame.area());

    let items: Vec<ListItem> = app
        .panes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let marker = if p.active && p.window_active {
                "●"
            } else {
                " "
            };
            let line = format!(
                "{marker} {}:{}.{}  {:<12}  {:<12}  {}x{}",
                p.session,
                p.window_index,
                p.pane_index,
                p.window_name,
                p.current_command,
                p.width,
                p.height
            );
            let style = if i == app.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            ListItem::new(line).style(style)
        })
        .collect();

    let title = format!(
        " HELM — session: {} ({} panes) ",
        app.session,
        app.panes.len()
    );
    frame.render_widget(
        List::new(items).block(Block::default().borders(Borders::ALL).title(title)),
        body,
    );
    frame.render_widget(
        Paragraph::new(" ↑/↓ select · Enter open · r refresh · q quit").dim(),
        help,
    );
}

fn draw_pane(frame: &mut Frame, app: &mut App) {
    let [grid_area, buttons_area, input_area, status_area] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(frame.area());

    // Record the inner viewport so paging/fit know the real size.
    let inner = Rect {
        x: grid_area.x + 1,
        y: grid_area.y + 1,
        width: grid_area.width.saturating_sub(2),
        height: grid_area.height.saturating_sub(2),
    };
    app.grid_viewport = (inner.width, inner.height);

    let title = match app.selected_pane() {
        Some(p) => format!(
            " {}:{}.{} {} — {} [{}x{}]{} ",
            p.session,
            p.window_index,
            p.pane_index,
            p.id,
            p.current_command,
            p.width,
            p.height,
            if app.scroll_offset > 0 {
                format!("  (scrolled -{})", app.scroll_offset)
            } else {
                String::new()
            }
        ),
        None => " (no pane) ".to_string(),
    };

    let lines = grid_lines(app, inner.height, inner.width);
    frame.render_widget(
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(title)),
        grid_area,
    );

    // Button row: [F2 Yes] [F3 No] …
    let mut spans: Vec<Span> = Vec::new();
    for (i, b) in app.buttons.iter().enumerate() {
        spans.push(Span::styled(
            format!("[F{} {}]", i + 2, b.label),
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Cyan),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw("[F8 Fit] [PgUp/PgDn scroll]").dim());
    frame.render_widget(Paragraph::new(Line::from(spans)), buttons_area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("> ", Style::default().fg(Color::Green)),
            Span::raw(app.input.as_str()),
            Span::styled("█", Style::default().add_modifier(Modifier::SLOW_BLINK)),
        ])),
        input_area,
    );

    let hint = if app.status.is_empty() {
        "Esc back · Enter send · Ctrl-q quit".to_string()
    } else {
        app.status.clone()
    };
    frame.render_widget(Paragraph::new(hint).dim(), status_area);
}

/// Build styled lines for the visible window of the grid.
fn grid_lines(app: &App, viewport_rows: u16, viewport_cols: u16) -> Vec<Line<'static>> {
    let Some(grid) = &app.grid else {
        return vec![Line::from("(connecting…)".dim())];
    };
    let (start, end) = app.visible_row_range(viewport_rows);
    let mut lines = Vec::with_capacity((end - start) as usize);
    for row in start..end {
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut run = String::new();
        let mut run_style = Style::default();
        for col in 0..grid.cols.min(viewport_cols) {
            let Some(cell) = grid.cell(col, row) else {
                continue;
            };
            if cell.attrs.wide_continuation() {
                continue;
            }
            let mut style = cell_style(cell);
            if grid.cursor == Some((col, row)) {
                style = style.add_modifier(Modifier::REVERSED);
            }
            if style != run_style && !run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut run), run_style));
            }
            run_style = style;
            run.push(cell.ch);
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, run_style));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn cell_style(cell: &Cell) -> Style {
    let mut style = Style::default();
    if let Some(fg) = map_color(cell.fg) {
        style = style.fg(fg);
    }
    if let Some(bg) = map_color(cell.bg) {
        style = style.bg(bg);
    }
    for (flag, m) in [
        (CellAttrs::BOLD, Modifier::BOLD),
        (CellAttrs::DIM, Modifier::DIM),
        (CellAttrs::ITALIC, Modifier::ITALIC),
        (CellAttrs::UNDERLINE, Modifier::UNDERLINED),
        (CellAttrs::REVERSE, Modifier::REVERSED),
        (CellAttrs::STRIKE, Modifier::CROSSED_OUT),
        (CellAttrs::BLINK, Modifier::SLOW_BLINK),
    ] {
        if cell.attrs.has(flag) {
            style = style.add_modifier(m);
        }
    }
    style
}

fn map_color(c: GridColor) -> Option<Color> {
    match c {
        GridColor::Default => None,
        GridColor::Indexed(n) => Some(Color::Indexed(n)),
        GridColor::Rgb(r, g, b) => Some(Color::Rgb(r, g, b)),
    }
}
