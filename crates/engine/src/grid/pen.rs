//! Shared SGR "pen" state + parameter application, used by both grid
//! producers: the capture-pane fast path (string params) and the VT screen
//! (vte `Params`). Both flatten to the same `u16` token stream — `38;5;196`
//! and `38:5:196` become `[38, 5, 196]`.

use super::{CellAttrs, Color};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub attrs: CellAttrs,
}

/// Apply one SGR sequence's parameters. An empty slice means reset (`ESC[m`).
pub fn apply_sgr_tokens(toks: &[u16], pen: &mut Pen) {
    if toks.is_empty() {
        *pen = Pen::default();
        return;
    }
    let mut i = 0;
    while i < toks.len() {
        let p = toks[i];
        match p {
            0 => *pen = Pen::default(),
            1 => pen.attrs.set(CellAttrs::BOLD, true),
            2 => pen.attrs.set(CellAttrs::DIM, true),
            3 => pen.attrs.set(CellAttrs::ITALIC, true),
            4 => pen.attrs.set(CellAttrs::UNDERLINE, true),
            5 => pen.attrs.set(CellAttrs::BLINK, true),
            7 => pen.attrs.set(CellAttrs::REVERSE, true),
            9 => pen.attrs.set(CellAttrs::STRIKE, true),
            21 | 22 => {
                pen.attrs.set(CellAttrs::BOLD, false);
                pen.attrs.set(CellAttrs::DIM, false);
            }
            23 => pen.attrs.set(CellAttrs::ITALIC, false),
            24 => pen.attrs.set(CellAttrs::UNDERLINE, false),
            25 => pen.attrs.set(CellAttrs::BLINK, false),
            27 => pen.attrs.set(CellAttrs::REVERSE, false),
            29 => pen.attrs.set(CellAttrs::STRIKE, false),
            30..=37 => pen.fg = Color::Indexed((p - 30) as u8),
            39 => pen.fg = Color::Default,
            40..=47 => pen.bg = Color::Indexed((p - 40) as u8),
            49 => pen.bg = Color::Default,
            90..=97 => pen.fg = Color::Indexed((p - 90 + 8) as u8),
            100..=107 => pen.bg = Color::Indexed((p - 100 + 8) as u8),
            38 | 48 => {
                let is_fg = p == 38;
                match toks.get(i + 1) {
                    Some(5) => {
                        let idx = toks.get(i + 2).copied().unwrap_or(0).min(255) as u8;
                        let c = Color::Indexed(idx);
                        if is_fg {
                            pen.fg = c;
                        } else {
                            pen.bg = c;
                        }
                        i += 2;
                    }
                    Some(2) => {
                        let c = |k: usize| toks.get(i + k).copied().unwrap_or(0).min(255) as u8;
                        let rgb = Color::Rgb(c(2), c(3), c(4));
                        if is_fg {
                            pen.fg = rgb;
                        } else {
                            pen.bg = rgb;
                        }
                        i += 4;
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        i += 1;
    }
}
