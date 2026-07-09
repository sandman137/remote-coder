//! VT screen (DESIGN.md §5, streaming path): interprets the raw pane byte
//! stream from control-mode `%output` into a live grid. Parsing is `vte`'s
//! job; this module implements the semantics: cursor motion, erase/insert/
//! delete, scroll regions, alt screen, autowrap, save/restore.
//!
//! Deliberately not implemented (agent TUIs don't need them): origin mode,
//! tab stops beyond every-8, character sets, mouse/paste modes (ignored).
//! If a real agent misrenders, escalate to `alacritty_terminal` per design.

use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

use super::pen::{apply_sgr_tokens, Pen};
use super::{Cell, CellAttrs, GridSnapshot};

pub struct VtScreen {
    parser: Parser,
    screen: Screen,
}

impl VtScreen {
    pub fn new(cols: u16, rows: u16) -> Self {
        VtScreen {
            parser: Parser::new(),
            screen: Screen::new(cols.max(1), rows.max(1)),
        }
    }

    /// Feed raw (already octal-unescaped) pane bytes.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }

    pub fn snapshot(&self) -> GridSnapshot {
        self.screen.snapshot()
    }

    pub fn size(&self) -> (u16, u16) {
        (self.screen.cols, self.screen.rows)
    }

    /// Resize without reflow (top-left anchored). Callers re-prime from
    /// `capture-pane` afterwards for fidelity — tmux is the reflow authority.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.screen.resize(cols.max(1), rows.max(1));
    }

    /// Reset and load a captured screen: `capture-pane -e` lines plus the
    /// cursor position — how streaming starts from current content instead
    /// of a blank grid (attach + post-resize priming).
    pub fn prime(&mut self, capture: &[u8], cursor: (u16, u16)) {
        self.screen.full_reset();
        let mut lines: Vec<&[u8]> = capture.split(|&b| b == b'\n').collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let rows = self.screen.rows as usize;
        for (i, line) in lines.iter().take(rows).enumerate() {
            self.feed(line);
            if i + 1 < lines.len().min(rows) {
                self.feed(b"\r\n");
            }
        }
        let (cx, cy) = cursor;
        self.feed(format!("\x1b[{};{}H", cy + 1, cx + 1).as_bytes());
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct SavedCursor {
    col: u16,
    row: u16,
    pen: Pen,
}

struct Screen {
    cols: u16,
    rows: u16,
    primary: Vec<Cell>,
    alt: Vec<Cell>,
    on_alt: bool,
    col: u16,
    row: u16,
    pen: Pen,
    wrap_pending: bool,
    autowrap: bool,
    cursor_visible: bool,
    saved_primary: SavedCursor,
    saved_alt: SavedCursor,
    /// Scroll margins, inclusive rows.
    scroll_top: u16,
    scroll_bottom: u16,
}

impl Screen {
    fn new(cols: u16, rows: u16) -> Self {
        let blank = vec![Cell::default(); cols as usize * rows as usize];
        Screen {
            cols,
            rows,
            primary: blank.clone(),
            alt: blank,
            on_alt: false,
            col: 0,
            row: 0,
            pen: Pen::default(),
            wrap_pending: false,
            autowrap: true,
            cursor_visible: true,
            saved_primary: SavedCursor::default(),
            saved_alt: SavedCursor::default(),
            scroll_top: 0,
            scroll_bottom: rows - 1,
        }
    }

    fn full_reset(&mut self) {
        *self = Screen::new(self.cols, self.rows);
    }

    fn snapshot(&self) -> GridSnapshot {
        let cells = if self.on_alt {
            self.alt.clone()
        } else {
            self.primary.clone()
        };
        GridSnapshot {
            cols: self.cols,
            rows: self.rows,
            cells,
            cursor: self
                .cursor_visible
                .then_some((self.col.min(self.cols - 1), self.row.min(self.rows - 1))),
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let copy = |old: &[Cell], oc: u16| -> Vec<Cell> {
            let mut new = vec![Cell::default(); cols as usize * rows as usize];
            for r in 0..rows.min(self.rows) {
                for c in 0..cols.min(oc) {
                    new[r as usize * cols as usize + c as usize] =
                        old[r as usize * oc as usize + c as usize];
                }
            }
            new
        };
        self.primary = copy(&self.primary, self.cols);
        self.alt = copy(&self.alt, self.cols);
        self.cols = cols;
        self.rows = rows;
        self.col = self.col.min(cols - 1);
        self.row = self.row.min(rows - 1);
        self.scroll_top = 0;
        self.scroll_bottom = rows - 1;
        self.wrap_pending = false;
    }

    fn buf(&mut self) -> &mut Vec<Cell> {
        if self.on_alt {
            &mut self.alt
        } else {
            &mut self.primary
        }
    }

    fn idx(&self, col: u16, row: u16) -> usize {
        row as usize * self.cols as usize + col as usize
    }

    /// Erased cells keep the current background (BCE), like xterm/alacritty.
    fn erase_cell(&self) -> Cell {
        Cell {
            ch: ' ',
            fg: crate::grid::Color::Default,
            bg: self.pen.bg,
            attrs: CellAttrs::default(),
        }
    }

    fn set_cell(&mut self, col: u16, row: u16, cell: Cell) {
        if col < self.cols && row < self.rows {
            let i = self.idx(col, row);
            self.buf()[i] = cell;
        }
    }

    fn clear_range(&mut self, from: usize, to: usize) {
        let cell = self.erase_cell();
        let len = self.buf().len();
        let (from, to) = (from.min(len), to.min(len));
        for c in &mut self.buf()[from..to] {
            *c = cell;
        }
    }

    fn scroll_up(&mut self, n: u16) {
        let n = n.max(1).min(self.scroll_bottom - self.scroll_top + 1);
        let cols = self.cols as usize;
        let top = self.scroll_top as usize * cols;
        let bottom = (self.scroll_bottom as usize + 1) * cols;
        let shift = n as usize * cols;
        self.buf().copy_within(top + shift..bottom, top);
        self.clear_range(bottom - shift, bottom);
    }

    fn scroll_down(&mut self, n: u16) {
        let n = n.max(1).min(self.scroll_bottom - self.scroll_top + 1);
        let cols = self.cols as usize;
        let top = self.scroll_top as usize * cols;
        let bottom = (self.scroll_bottom as usize + 1) * cols;
        let shift = n as usize * cols;
        self.buf().copy_within(top..bottom - shift, top + shift);
        self.clear_range(top, top + shift);
    }

    fn linefeed(&mut self) {
        if self.row == self.scroll_bottom {
            self.scroll_up(1);
        } else if self.row + 1 < self.rows {
            self.row += 1;
        }
    }

    fn reverse_linefeed(&mut self) {
        if self.row == self.scroll_top {
            self.scroll_down(1);
        } else {
            self.row = self.row.saturating_sub(1);
        }
    }

    fn carriage_return(&mut self) {
        self.col = 0;
        self.wrap_pending = false;
    }

    fn put_char(&mut self, ch: char) {
        let width = ch.width().unwrap_or(0) as u16;
        if width == 0 {
            return; // combining marks: dropped (see module docs)
        }
        if self.wrap_pending && self.autowrap {
            self.carriage_return();
            self.linefeed();
        }
        self.wrap_pending = false;

        // A wide char that can't fit in the remaining columns wraps early.
        if width == 2 && self.col + 1 >= self.cols {
            if self.autowrap {
                self.carriage_return();
                self.linefeed();
            } else {
                self.col = self.cols.saturating_sub(2);
            }
        }

        let cell = Cell {
            ch,
            fg: self.pen.fg,
            bg: self.pen.bg,
            attrs: self.pen.attrs,
        };
        self.set_cell(self.col, self.row, cell);
        if width == 2 {
            let mut attrs = self.pen.attrs;
            attrs.set(CellAttrs::WIDE_CONT, true);
            self.set_cell(
                self.col + 1,
                self.row,
                Cell {
                    ch: ' ',
                    fg: self.pen.fg,
                    bg: self.pen.bg,
                    attrs,
                },
            );
        }

        if self.col + width >= self.cols {
            self.col = self.cols - 1;
            self.wrap_pending = true;
        } else {
            self.col += width;
        }
    }

    fn move_to(&mut self, col: u16, row: u16) {
        self.col = col.min(self.cols - 1);
        self.row = row.min(self.rows - 1);
        self.wrap_pending = false;
    }

    fn save_cursor(&mut self) {
        let saved = SavedCursor {
            col: self.col,
            row: self.row,
            pen: self.pen,
        };
        if self.on_alt {
            self.saved_alt = saved;
        } else {
            self.saved_primary = saved;
        }
    }

    fn restore_cursor(&mut self) {
        let saved = if self.on_alt {
            self.saved_alt
        } else {
            self.saved_primary
        };
        self.col = saved.col.min(self.cols - 1);
        self.row = saved.row.min(self.rows - 1);
        self.pen = saved.pen;
        self.wrap_pending = false;
    }

    fn enter_alt(&mut self, clear: bool, save_cursor: bool) {
        if save_cursor {
            self.save_cursor();
        }
        if !self.on_alt {
            self.on_alt = true;
        }
        if clear {
            let blank = self.erase_cell();
            for c in &mut self.alt {
                *c = blank;
            }
            self.move_to(0, 0);
        }
    }

    fn exit_alt(&mut self, restore_cursor: bool) {
        if self.on_alt {
            self.on_alt = false;
        }
        if restore_cursor {
            self.restore_cursor();
        }
    }

    fn set_private_mode(&mut self, mode: u16, on: bool) {
        match mode {
            7 => self.autowrap = on,
            25 => self.cursor_visible = on,
            47 => {
                if on {
                    self.enter_alt(false, false)
                } else {
                    self.exit_alt(false)
                }
            }
            1047 => {
                if on {
                    self.enter_alt(true, false)
                } else {
                    self.exit_alt(false)
                }
            }
            1048 => {
                if on {
                    self.save_cursor()
                } else {
                    self.restore_cursor()
                }
            }
            1049 => {
                if on {
                    self.enter_alt(true, true)
                } else {
                    self.exit_alt(true)
                }
            }
            _ => {} // mouse/paste/etc: ignore
        }
    }
}

/// First param with 0→default(1) semantics for cursor-motion CSIs.
fn p1(params: &[u16]) -> u16 {
    params.first().copied().filter(|&v| v != 0).unwrap_or(1)
}

impl Perform for Screen {
    fn print(&mut self, c: char) {
        self.put_char(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x08 => {
                // BS
                self.col = self.col.saturating_sub(1);
                self.wrap_pending = false;
            }
            0x09 => {
                // HT: next multiple-of-8 stop
                self.col = ((self.col / 8 + 1) * 8).min(self.cols - 1);
                self.wrap_pending = false;
            }
            0x0a..=0x0c => self.linefeed(), // LF, VT, FF
            0x0d => self.carriage_return(),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let flat: Vec<u16> = params.iter().flatten().copied().collect();
        let private = intermediates.first() == Some(&b'?');

        if private {
            match action {
                'h' => {
                    for &m in &flat {
                        self.set_private_mode(m, true);
                    }
                }
                'l' => {
                    for &m in &flat {
                        self.set_private_mode(m, false);
                    }
                }
                _ => {}
            }
            return;
        }

        match action {
            'A' => self.move_to(self.col, self.row.saturating_sub(p1(&flat))),
            'B' | 'e' => self.move_to(self.col, self.row.saturating_add(p1(&flat))),
            'C' | 'a' => self.move_to(self.col.saturating_add(p1(&flat)), self.row),
            'D' => self.move_to(self.col.saturating_sub(p1(&flat)), self.row),
            'E' => self.move_to(0, self.row.saturating_add(p1(&flat))),
            'F' => self.move_to(0, self.row.saturating_sub(p1(&flat))),
            'G' | '`' => self.move_to(p1(&flat) - 1, self.row),
            'd' => self.move_to(self.col, p1(&flat) - 1),
            'H' | 'f' => {
                let row = p1(&flat) - 1;
                let col = flat.get(1).copied().filter(|&v| v != 0).unwrap_or(1) - 1;
                self.move_to(col, row);
            }
            'J' => {
                let mode = flat.first().copied().unwrap_or(0);
                let cur = self.idx(self.col, self.row);
                let end = self.idx(self.cols - 1, self.rows - 1) + 1;
                match mode {
                    0 => self.clear_range(cur, end),
                    1 => self.clear_range(0, cur + 1),
                    2 | 3 => self.clear_range(0, end),
                    _ => {}
                }
            }
            'K' => {
                let mode = flat.first().copied().unwrap_or(0);
                let line = self.idx(0, self.row);
                let cur = self.idx(self.col, self.row);
                match mode {
                    0 => self.clear_range(cur, line + self.cols as usize),
                    1 => self.clear_range(line, cur + 1),
                    2 => self.clear_range(line, line + self.cols as usize),
                    _ => {}
                }
            }
            'L' => {
                // IL: insert lines at cursor (inside margins)
                if self.row >= self.scroll_top && self.row <= self.scroll_bottom {
                    let n = p1(&flat);
                    let saved_top = self.scroll_top;
                    self.scroll_top = self.row;
                    self.scroll_down(n);
                    self.scroll_top = saved_top;
                }
            }
            'M' => {
                // DL: delete lines at cursor
                if self.row >= self.scroll_top && self.row <= self.scroll_bottom {
                    let n = p1(&flat);
                    let saved_top = self.scroll_top;
                    self.scroll_top = self.row;
                    self.scroll_up(n);
                    self.scroll_top = saved_top;
                }
            }
            'P' => {
                // DCH: delete chars, shift rest of line left
                let n = p1(&flat).min(self.cols - self.col) as usize;
                let line_start = self.idx(0, self.row);
                let cur = self.idx(self.col, self.row);
                let line_end = line_start + self.cols as usize;
                self.buf().copy_within(cur + n..line_end, cur);
                self.clear_range(line_end - n, line_end);
            }
            '@' => {
                // ICH: insert blanks, shift right
                let n = p1(&flat).min(self.cols - self.col) as usize;
                let line_start = self.idx(0, self.row);
                let cur = self.idx(self.col, self.row);
                let line_end = line_start + self.cols as usize;
                self.buf().copy_within(cur..line_end - n, cur + n);
                self.clear_range(cur, cur + n);
            }
            'X' => {
                // ECH: erase n chars in place
                let n = p1(&flat).min(self.cols - self.col) as usize;
                let cur = self.idx(self.col, self.row);
                self.clear_range(cur, cur + n);
            }
            'S' => self.scroll_up(p1(&flat)),
            'T' => self.scroll_down(p1(&flat)),
            'r' => {
                let top = p1(&flat) - 1;
                let bottom = flat
                    .get(1)
                    .copied()
                    .filter(|&v| v != 0)
                    .unwrap_or(self.rows)
                    - 1;
                if top < bottom && bottom < self.rows {
                    self.scroll_top = top;
                    self.scroll_bottom = bottom;
                    self.move_to(0, 0);
                }
            }
            'm' => apply_sgr_tokens(&flat, &mut self.pen),
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if !intermediates.is_empty() {
            return; // charset selection etc.
        }
        match byte {
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'D' => self.linefeed(),
            b'E' => {
                self.carriage_return();
                self.linefeed();
            }
            b'M' => self.reverse_linefeed(),
            b'c' => self.full_reset(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::Color;

    fn screen(cols: u16, rows: u16, feeds: &[&[u8]]) -> VtScreen {
        let mut vt = VtScreen::new(cols, rows);
        for f in feeds {
            vt.feed(f);
        }
        vt
    }

    #[test]
    fn plain_lines_and_cursor() {
        let vt = screen(10, 3, &[b"ab\r\ncd"]);
        let g = vt.snapshot();
        assert_eq!(g.row_text(0), "ab");
        assert_eq!(g.row_text(1), "cd");
        assert_eq!(g.cursor, Some((2, 1)));
    }

    #[test]
    fn scrolls_at_bottom() {
        let vt = screen(10, 2, &[b"1\r\n2\r\n3"]);
        let g = vt.snapshot();
        assert_eq!(g.row_text(0), "2");
        assert_eq!(g.row_text(1), "3");
    }

    #[test]
    fn autowrap_defers_until_next_char() {
        let vt = screen(4, 2, &[b"abcd", b"e"]);
        let g = vt.snapshot();
        assert_eq!(g.row_text(0), "abcd");
        assert_eq!(g.row_text(1), "e");
        assert_eq!(g.cursor, Some((1, 1)));
    }

    #[test]
    fn cursor_up_rewrite_line() {
        // The fake-stream pattern: ESC[1A CR ESC[K rewrite.
        let vt = screen(
            20,
            4,
            &[b"tick 1\r\ntick 2\r\n", b"\x1b[1A\r\x1b[Krewritten\r\n"],
        );
        let g = vt.snapshot();
        assert_eq!(g.row_text(0), "tick 1");
        assert_eq!(g.row_text(1), "rewritten");
    }

    #[test]
    fn cup_addresses_absolutely() {
        let vt = screen(20, 6, &[b"\x1b[3;5HX"]);
        let g = vt.snapshot();
        assert_eq!(g.cell(4, 2).unwrap().ch, 'X');
        assert_eq!(g.cursor, Some((5, 2)));
    }

    #[test]
    fn alt_screen_round_trip() {
        let vt = screen(
            20,
            4,
            &[
                b"primary\r\n",
                b"\x1b[?1049h\x1b[2J\x1b[H\x1b[2;3Halt!",
                b"\x1b[?1049l",
            ],
        );
        let g = vt.snapshot();
        assert_eq!(g.row_text(0), "primary");
        assert!(!g.to_text().contains("alt!"));
        // Cursor restored to where it was before entering alt.
        assert_eq!(g.cursor, Some((0, 1)));
    }

    #[test]
    fn alt_screen_contents_while_active() {
        let vt = screen(20, 4, &[b"primary\r\n", b"\x1b[?1049h\x1b[2J\x1b[2;3Halt!"]);
        let g = vt.snapshot();
        assert_eq!(g.row_text(1), "  alt!");
        assert!(!g.to_text().contains("primary"));
    }

    #[test]
    fn colors_flow_through_shared_pen() {
        let vt = screen(20, 2, &[b"\x1b[31;1mR\x1b[0m\x1b[38;2;1;2;3mT"]);
        let g = vt.snapshot();
        assert_eq!(g.cell(0, 0).unwrap().fg, Color::Indexed(1));
        assert!(g.cell(0, 0).unwrap().attrs.bold());
        assert_eq!(g.cell(1, 0).unwrap().fg, Color::Rgb(1, 2, 3));
    }

    #[test]
    fn erase_line_variants() {
        let vt = screen(10, 1, &[b"abcdefgh", b"\x1b[5G\x1b[1K"]);
        let g = vt.snapshot();
        // EL 1 erases through the cursor column (5th col, index 4).
        assert_eq!(g.row_text(0), "     fgh");
    }

    #[test]
    fn scroll_region_contains_scrolling() {
        let vt = screen(10, 5, &[b"top\r\n", b"\x1b[2;4r", b"\x1b[4;1Ha\r\nb\r\nc"]);
        let g = vt.snapshot();
        // Row 0 stays; rows 1..=3 scrolled within the margin.
        assert_eq!(g.row_text(0), "top");
        assert_eq!(g.row_text(3), "c");
    }

    #[test]
    fn insert_delete_chars() {
        let vt = screen(10, 1, &[b"abcdef", b"\x1b[3G\x1b[2@XY"]);
        let g = vt.snapshot();
        assert_eq!(g.row_text(0), "abXYcdef");
        let vt2 = screen(10, 1, &[b"abcdef", b"\x1b[3G\x1b[2P"]);
        assert_eq!(vt2.snapshot().row_text(0), "abef");
    }

    #[test]
    fn wide_chars_and_wrap() {
        let vt = screen(5, 2, &[b"abcd", "日".as_bytes()]);
        let g = vt.snapshot();
        // 日 needs 2 cols but only 1 remains → wraps to next line.
        assert_eq!(g.row_text(0), "abcd");
        assert_eq!(g.row_text(1), "日");
        assert!(g.cell(1, 1).unwrap().attrs.wide_continuation());
    }

    #[test]
    fn cursor_hide_show() {
        let mut vt = screen(5, 2, &[b"\x1b[?25l"]);
        assert_eq!(vt.snapshot().cursor, None);
        vt.feed(b"\x1b[?25h");
        assert!(vt.snapshot().cursor.is_some());
    }

    #[test]
    fn prime_reconstructs_capture() {
        let mut vt = VtScreen::new(20, 4);
        vt.prime(b"\x1b[32mok\x1b[0m line\nsecond\n", (3, 1));
        let g = vt.snapshot();
        assert_eq!(g.row_text(0), "ok line");
        assert_eq!(g.row_text(1), "second");
        assert_eq!(g.cell(0, 0).unwrap().fg, Color::Indexed(2));
        assert_eq!(g.cursor, Some((3, 1)));
        // Priming again resets prior content.
        vt.prime(b"fresh\n", (0, 0));
        let g2 = vt.snapshot();
        assert_eq!(g2.row_text(0), "fresh");
        assert!(!g2.to_text().contains("second"));
    }

    #[test]
    fn resize_preserves_top_left_and_clamps() {
        let mut vt = screen(10, 4, &[b"abcdefgh\r\nline2"]);
        vt.resize(5, 2);
        let g = vt.snapshot();
        assert_eq!((g.cols, g.rows), (5, 2));
        assert_eq!(g.row_text(0), "abcde");
        assert_eq!(g.row_text(1), "line2");
        assert!(g.cursor.unwrap().0 < 5);
    }
}
