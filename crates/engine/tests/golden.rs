//! Golden tests (DESIGN.md §10.1 layer 2): recorded/hand-crafted control-mode
//! streams replayed through the notification parser + VT grid, asserting
//! exact grids. Deterministic, no tmux needed — CI-safe.
//!
//! Fixture payloads are octal-escaped exactly as tmux emits them, so these
//! cover the §4.2 unescape requirement end to end.

use std::path::PathBuf;

use engine::grid::vt::VtScreen;
use engine::tmux::control::{ControlEvent, ControlParser};
use engine::{Color, GridSnapshot};

fn replay(fixture: &str, cols: u16, rows: u16) -> GridSnapshot {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/control-mode")
        .join(fixture);
    let data = std::fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));

    let mut parser = ControlParser::new();
    let mut screen = VtScreen::new(cols, rows);
    for line in data.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        if let ControlEvent::Output { pane, bytes } = parser.feed_line(line) {
            assert_eq!(pane.as_str(), "%0", "single-pane fixtures");
            screen.feed(&bytes);
        }
    }
    screen.snapshot()
}

#[test]
fn octal_color_stream_renders_exactly() {
    let g = replay("octal-color.stream", 80, 24);

    assert_eq!(g.row_text(0), "plain text");
    assert_eq!(g.row_text(1), "red and bright");
    assert_eq!(g.row_text(2), "café \\ backslash");
    assert_eq!(g.row_text(3), "truecolor");

    // "red" in basic red, " and " default, "bright" in 256-color 196.
    assert_eq!(g.cell(0, 1).unwrap().fg, Color::Indexed(1));
    assert_eq!(g.cell(2, 1).unwrap().fg, Color::Indexed(1));
    assert_eq!(g.cell(4, 1).unwrap().fg, Color::Default);
    assert_eq!(g.cell(8, 1).unwrap().fg, Color::Indexed(196));
    assert_eq!(g.cell(13, 1).unwrap().fg, Color::Indexed(196));

    // Truecolor row.
    assert_eq!(g.cell(0, 3).unwrap().fg, Color::Rgb(255, 128, 0));
    assert_eq!(g.cell(8, 3).unwrap().fg, Color::Rgb(255, 128, 0));

    // Cursor parked after "truecolor".
    assert_eq!(g.cursor, Some((9, 3)));
}

#[test]
fn altscreen_active_shows_alt_content() {
    let g = replay("altscreen-active.stream", 80, 24);

    // Alt screen: CUP 3;5 wrote at row 2, col 4; reverse "status" at origin.
    assert_eq!(g.row_text(2), "    ALT-SCREEN-UI");
    assert_eq!(g.row_text(0), "status");
    assert!(g.cell(0, 0).unwrap().attrs.reverse());
    assert!(!g.cell(6, 0).unwrap().attrs.reverse());
    // Primary content must not bleed through.
    assert!(!g.to_text().contains("shell prompt"));
}

#[test]
fn altscreen_roundtrip_restores_primary() {
    let g = replay("altscreen-roundtrip.stream", 80, 24);

    assert_eq!(g.row_text(0), "shell prompt $");
    assert!(!g.to_text().contains("ALT-SCREEN-UI"));
    // Cursor restored to where it was when 1049h saved it.
    assert_eq!(g.cursor, Some((0, 1)));
}

#[test]
fn cursor_rewrite_stream() {
    let g = replay("cursor-rewrite.stream", 80, 24);

    assert_eq!(g.row_text(0), "tick 1");
    assert_eq!(g.row_text(1), "rewritten line");
    assert_eq!(g.row_text(2), "● done");
    assert_eq!(g.cell(0, 2).unwrap().fg, Color::Indexed(6));
    assert_eq!(g.cell(1, 2).unwrap().fg, Color::Default);
    assert_eq!(g.cursor, Some((6, 2)));
}

#[test]
fn wide_unicode_stream() {
    let g = replay("wide-unicode.stream", 80, 24);

    assert_eq!(g.row_text(0), "日本語 CJK");
    assert_eq!(g.row_text(1), "emoji: 🚀!");

    // Wide chars occupy two cells with continuation markers.
    assert_eq!(g.cell(0, 0).unwrap().ch, '日');
    assert!(g.cell(1, 0).unwrap().attrs.wide_continuation());
    assert_eq!(g.cell(2, 0).unwrap().ch, '本');
    assert_eq!(g.cell(4, 0).unwrap().ch, '語');
    assert_eq!(g.cell(6, 0).unwrap().ch, ' ');
    assert_eq!(g.cell(7, 0).unwrap().ch, 'C');

    assert_eq!(g.cell(7, 1).unwrap().ch, '🚀');
    assert!(g.cell(8, 1).unwrap().attrs.wide_continuation());
    assert_eq!(g.cell(9, 1).unwrap().ch, '!');
}
