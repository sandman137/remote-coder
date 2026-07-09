//! SGR fast path (DESIGN.md §5): parse `capture-pane -e` output into a
//! `GridSnapshot`. tmux has already resolved cursor motion into a grid of
//! lines, so the only escapes to interpret are `\e[…m` (colors/attributes) —
//! this is the literal realization of "no terminal emulator". Any other
//! escape sequence is skipped defensively.
//!
//! Unicode: wide chars occupy two cells (the second marked `WIDE_CONT`);
//! zero-width/combining marks are dropped on this path (the Phase-3 VT grid
//! owns full fidelity).

use unicode_width::UnicodeWidthChar;

use super::pen::{apply_sgr_tokens, Pen};
use super::{Cell, CellAttrs, GridSnapshot};

/// Parse captured lines into a `cols`-wide grid with at least `min_rows` rows
/// (padded with blanks at the bottom, as tmux omits trailing blank lines).
/// Lines beyond `cols` are clipped — capture width should equal pane width.
pub fn parse_capture(raw: &[u8], cols: u16, min_rows: u16) -> GridSnapshot {
    let text = String::from_utf8_lossy(raw);
    let lines: Vec<&str> = if text.is_empty() {
        Vec::new()
    } else {
        let mut v: Vec<&str> = text.split('\n').collect();
        if v.last() == Some(&"") {
            v.pop(); // trailing newline
        }
        v
    };

    let rows = (lines.len() as u16).max(min_rows);
    let mut grid = GridSnapshot::new(cols, rows);
    let mut pen = Pen::default();

    for (row, line) in lines.iter().enumerate() {
        parse_line(line, &mut pen, &mut grid, row as u16);
    }
    grid
}

fn parse_line(line: &str, pen: &mut Pen, grid: &mut GridSnapshot, row: u16) {
    let mut col: u16 = 0;
    let mut chars = line.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            match chars.peek() {
                Some('[') => {
                    chars.next();
                    // CSI: params until a final byte 0x40..=0x7e.
                    let mut params = String::new();
                    let mut fin = '\0';
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            fin = c;
                            break;
                        }
                        params.push(c);
                    }
                    if fin == 'm' {
                        apply_sgr(&params, pen);
                    }
                    // Non-SGR CSI (shouldn't appear in capture output): skip.
                }
                Some(']') => {
                    // OSC: skip until BEL or ST.
                    chars.next();
                    let mut prev = '\0';
                    for c in chars.by_ref() {
                        if c == '\u{7}' || (prev == '\u{1b}' && c == '\\') {
                            break;
                        }
                        prev = c;
                    }
                }
                _ => {
                    // Other escape (e.g. charset select): skip one final char.
                    chars.next();
                }
            }
            continue;
        }

        let width = ch.width().unwrap_or(0);
        if width == 0 {
            continue; // combining/zero-width: dropped on the fast path
        }
        if col >= grid.cols {
            break; // clip
        }
        if width == 2 && col + 1 >= grid.cols {
            break; // wide char would straddle the edge: clip
        }

        if let Some(cell) = grid.cell_mut(col, row) {
            *cell = Cell {
                ch,
                fg: pen.fg,
                bg: pen.bg,
                attrs: pen.attrs,
            };
        }
        if width == 2 {
            if let Some(cont) = grid.cell_mut(col + 1, row) {
                *cont = Cell {
                    ch: ' ',
                    fg: pen.fg,
                    bg: pen.bg,
                    attrs: {
                        let mut a = pen.attrs;
                        a.set(CellAttrs::WIDE_CONT, true);
                        a
                    },
                };
            }
        }
        col += width as u16;
    }
}

/// Apply an SGR parameter string (`"1;38;5;196"`). Handles both `;` and `:`
/// separated extended-color forms by flattening to one token stream.
fn apply_sgr(params: &str, pen: &mut Pen) {
    if params.is_empty() {
        *pen = Pen::default();
        return;
    }
    let toks: Vec<u16> = params
        .split([';', ':'])
        .map(|p| p.parse::<u16>().unwrap_or(0))
        .collect();
    apply_sgr_tokens(&toks, pen);
}

#[cfg(test)]
mod tests {
    use super::super::Color;
    use super::*;

    fn cell(g: &GridSnapshot, col: u16, row: u16) -> Cell {
        *g.cell(col, row).unwrap()
    }

    #[test]
    fn plain_text_lands_in_cells() {
        let g = parse_capture(b"hi\nthere\n", 10, 3);
        assert_eq!((g.cols, g.rows), (10, 3));
        assert_eq!(g.row_text(0), "hi");
        assert_eq!(g.row_text(1), "there");
        assert_eq!(g.row_text(2), "");
    }

    #[test]
    fn basic_and_bright_colors() {
        let g = parse_capture(b"\x1b[31mr\x1b[0m \x1b[92mg\x1b[m", 10, 1);
        assert_eq!(cell(&g, 0, 0).fg, Color::Indexed(1));
        assert_eq!(cell(&g, 1, 0).fg, Color::Default);
        assert_eq!(cell(&g, 2, 0).fg, Color::Indexed(10));
    }

    #[test]
    fn indexed_256_and_truecolor_both_separators() {
        let g = parse_capture(
            b"\x1b[38;5;196ma\x1b[48;2;10;20;30mb\x1b[0m\x1b[38:5:21mc",
            10,
            1,
        );
        assert_eq!(cell(&g, 0, 0).fg, Color::Indexed(196));
        assert_eq!(cell(&g, 1, 0).bg, Color::Rgb(10, 20, 30));
        assert_eq!(cell(&g, 2, 0).fg, Color::Indexed(21));
        assert_eq!(cell(&g, 2, 0).bg, Color::Default);
    }

    #[test]
    fn attrs_set_and_clear() {
        let g = parse_capture(b"\x1b[1;7mx\x1b[22my\x1b[27mz", 10, 1);
        assert!(cell(&g, 0, 0).attrs.bold());
        assert!(cell(&g, 0, 0).attrs.reverse());
        assert!(!cell(&g, 1, 0).attrs.bold());
        assert!(cell(&g, 1, 0).attrs.reverse());
        assert!(!cell(&g, 2, 0).attrs.reverse());
    }

    #[test]
    fn pen_persists_across_lines() {
        // tmux capture keeps SGR state running across line boundaries.
        let g = parse_capture(b"\x1b[35mab\ncd\x1b[0m\n", 10, 2);
        assert_eq!(cell(&g, 0, 1).fg, Color::Indexed(5));
    }

    #[test]
    fn wide_chars_take_two_cells() {
        let g = parse_capture("界x".as_bytes(), 10, 1);
        assert_eq!(cell(&g, 0, 0).ch, '界');
        assert!(cell(&g, 1, 0).attrs.wide_continuation());
        assert_eq!(cell(&g, 2, 0).ch, 'x');
        assert_eq!(g.row_text(0), "界x");
    }

    #[test]
    fn emoji_is_wide() {
        let g = parse_capture("🚀!".as_bytes(), 10, 1);
        assert_eq!(cell(&g, 0, 0).ch, '🚀');
        assert!(cell(&g, 1, 0).attrs.wide_continuation());
        assert_eq!(cell(&g, 2, 0).ch, '!');
    }

    #[test]
    fn long_lines_clip_at_cols() {
        let g = parse_capture(b"abcdefghij", 4, 1);
        assert_eq!(g.row_text(0), "abcd");
    }

    #[test]
    fn wide_char_at_edge_clips() {
        let g = parse_capture("abc界".as_bytes(), 4, 1);
        assert_eq!(g.row_text(0), "abc");
    }

    #[test]
    fn unknown_escapes_are_skipped() {
        // OSC title set + a non-SGR CSI must not leak into cells.
        let g = parse_capture(b"\x1b]0;title\x07ok\x1b[2Jx", 10, 1);
        assert_eq!(g.row_text(0), "okx");
    }
}
