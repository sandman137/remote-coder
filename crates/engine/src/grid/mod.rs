//! VT grid model (DESIGN.md §5). `GridSnapshot` is what UIs render and what
//! UniFFI exposes. Two producers: the SGR fast path (snapshot mode, this
//! phase) and the VT state machine (streaming mode, Phase 3).

pub mod pen;
pub mod sgr;
pub mod vt;

/// Terminal color: default / 256-indexed / truecolor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Compact cell attribute bitfield.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CellAttrs(u8);

impl CellAttrs {
    pub const BOLD: u8 = 1 << 0;
    pub const DIM: u8 = 1 << 1;
    pub const ITALIC: u8 = 1 << 2;
    pub const UNDERLINE: u8 = 1 << 3;
    pub const REVERSE: u8 = 1 << 4;
    pub const STRIKE: u8 = 1 << 5;
    pub const BLINK: u8 = 1 << 6;
    /// Continuation cell of a wide character (renderers skip it).
    pub const WIDE_CONT: u8 = 1 << 7;

    pub fn set(&mut self, flag: u8, on: bool) {
        if on {
            self.0 |= flag;
        } else {
            self.0 &= !flag;
        }
    }
    pub fn has(&self, flag: u8) -> bool {
        self.0 & flag != 0
    }
    pub fn bits(&self) -> u8 {
        self.0
    }
    pub fn from_bits(bits: u8) -> Self {
        CellAttrs(bits)
    }
    pub fn bold(&self) -> bool {
        self.has(Self::BOLD)
    }
    pub fn reverse(&self) -> bool {
        self.has(Self::REVERSE)
    }
    pub fn wide_continuation(&self) -> bool {
        self.has(Self::WIDE_CONT)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub attrs: CellAttrs,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::default(),
        }
    }
}

/// A dense cols×rows grid. Row-major: cell (col, row) = `cells[row*cols + col]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GridSnapshot {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<Cell>,
    /// Cursor (col, row), if known and inside the grid.
    pub cursor: Option<(u16, u16)>,
}

impl GridSnapshot {
    pub fn new(cols: u16, rows: u16) -> Self {
        GridSnapshot {
            cols,
            rows,
            cells: vec![Cell::default(); cols as usize * rows as usize],
            cursor: None,
        }
    }

    pub fn cell(&self, col: u16, row: u16) -> Option<&Cell> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.cells
            .get(row as usize * self.cols as usize + col as usize)
    }

    pub fn cell_mut(&mut self, col: u16, row: u16) -> Option<&mut Cell> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.cells
            .get_mut(row as usize * self.cols as usize + col as usize)
    }

    /// Plain text of one row, wide-continuation cells skipped, right-trimmed.
    pub fn row_text(&self, row: u16) -> String {
        let mut s = String::with_capacity(self.cols as usize);
        for col in 0..self.cols {
            if let Some(c) = self.cell(col, row) {
                if !c.attrs.wide_continuation() {
                    s.push(c.ch);
                }
            }
        }
        s.truncate(s.trim_end().len());
        s
    }

    /// Full plain text, one line per row (attention regexes match on this).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        for row in 0..self.rows {
            s.push_str(&self.row_text(row));
            s.push('\n');
        }
        s
    }

    /// Row indexes that differ from `prev` (or every row on shape change).
    pub fn dirty_rows(&self, prev: &GridSnapshot) -> Vec<u16> {
        if self.cols != prev.cols || self.rows != prev.rows {
            return (0..self.rows).collect();
        }
        let cols = self.cols as usize;
        (0..self.rows)
            .filter(|&r| {
                let a = r as usize * cols;
                self.cells[a..a + cols] != prev.cells[a..a + cols]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn row_text_trims_and_skips_continuations() {
        let mut g = GridSnapshot::new(6, 1);
        g.cell_mut(0, 0).unwrap().ch = '界';
        g.cell_mut(1, 0)
            .unwrap()
            .attrs
            .set(CellAttrs::WIDE_CONT, true);
        g.cell_mut(2, 0).unwrap().ch = 'x';
        assert_eq!(g.row_text(0), "界x");
    }

    #[test]
    fn dirty_rows_detects_changes() {
        let a = GridSnapshot::new(4, 3);
        let mut b = a.clone();
        b.cell_mut(2, 1).unwrap().ch = 'z';
        assert_eq!(b.dirty_rows(&a), vec![1]);
        let c = GridSnapshot::new(5, 3);
        assert_eq!(c.dirty_rows(&a), vec![0, 1, 2]);
    }
}
