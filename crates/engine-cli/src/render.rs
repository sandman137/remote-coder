//! Re-render a `GridSnapshot` as ANSI text (headless `snapshot --ansi`).

use engine::{Cell, CellAttrs, Color, GridSnapshot};

fn push_color(out: &mut Vec<String>, color: Color, fg: bool) {
    let base = if fg { 30 } else { 40 };
    match color {
        Color::Default => out.push((base + 9).to_string()),
        Color::Indexed(n) if n < 8 => out.push((base + n as u16).to_string()),
        Color::Indexed(n) if n < 16 => out.push((base + 60 + (n as u16 - 8)).to_string()),
        Color::Indexed(n) => out.push(format!("{};5;{n}", base + 8)),
        Color::Rgb(r, g, b) => out.push(format!("{};2;{r};{g};{b}", base + 8)),
    }
}

fn sgr_for(cell: &Cell) -> String {
    let mut params = vec!["0".to_string()];
    for (flag, code) in [
        (CellAttrs::BOLD, "1"),
        (CellAttrs::DIM, "2"),
        (CellAttrs::ITALIC, "3"),
        (CellAttrs::UNDERLINE, "4"),
        (CellAttrs::BLINK, "5"),
        (CellAttrs::REVERSE, "7"),
        (CellAttrs::STRIKE, "9"),
    ] {
        if cell.attrs.has(flag) {
            params.push(code.to_string());
        }
    }
    push_color(&mut params, cell.fg, true);
    push_color(&mut params, cell.bg, false);
    format!("\x1b[{}m", params.join(";"))
}

pub fn grid_to_ansi(grid: &GridSnapshot) -> String {
    let mut out = String::new();
    for row in 0..grid.rows {
        let mut pen: Option<String> = None;
        for col in 0..grid.cols {
            let Some(cell) = grid.cell(col, row) else {
                continue;
            };
            if cell.attrs.wide_continuation() {
                continue;
            }
            let sgr = sgr_for(cell);
            if pen.as_deref() != Some(&sgr) {
                out.push_str(&sgr);
                pen = Some(sgr);
            }
            out.push(cell.ch);
        }
        out.push_str("\x1b[0m\n");
    }
    out
}
