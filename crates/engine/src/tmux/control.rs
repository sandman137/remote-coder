//! Control-mode (`tmux -C`) notification parser (DESIGN.md §4.2).
//!
//! Lines are either `%`-notifications or, between `%begin`/`%end|%error`,
//! the output of a command we issued. `%output` payloads arrive
//! octal-escaped and are decoded here via [`super::vis_unescape`] — the
//! single most common control-mode correctness bug when missed.

use super::{vis_unescape, PaneId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlEvent {
    /// A command's reply block completed (`%end` ok, `%error` not).
    CommandDone {
        num: u64,
        ok: bool,
        output: Vec<String>,
    },
    /// Raw pane bytes (already unescaped) — feed the VT screen.
    Output {
        pane: PaneId,
        bytes: Vec<u8>,
    },
    /// Window geometry changed; `layout` is the tmux layout string.
    LayoutChange {
        window_id: String,
        layout: String,
    },
    /// Windows appeared/disappeared/renamed — re-enumerate panes.
    WindowsChanged,
    SessionChanged {
        id: String,
        name: String,
    },
    PaneModeChanged {
        pane: PaneId,
    },
    /// The control client is being detached (server exit, kill, detach).
    Exit {
        reason: Option<String>,
    },
    /// Recognized-but-uninteresting or unknown line.
    Ignored,
}

#[derive(Debug, Default)]
pub struct ControlParser {
    block: Option<(u64, Vec<String>)>,
}

impl ControlParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one raw line (without trailing newline).
    pub fn feed_line(&mut self, raw: &[u8]) -> ControlEvent {
        let raw = match raw.last() {
            Some(b'\r') => &raw[..raw.len() - 1],
            _ => raw,
        };

        // Inside a reply block, everything except %end/%error is output.
        if self.block.is_some() {
            if let Some(rest) = strip_prefix(raw, b"%end ") {
                return self.finish_block(rest, true);
            }
            if let Some(rest) = strip_prefix(raw, b"%error ") {
                return self.finish_block(rest, false);
            }
            let line = String::from_utf8_lossy(&vis_unescape(raw)).into_owned();
            if let Some((_, lines)) = &mut self.block {
                lines.push(line);
            }
            return ControlEvent::Ignored;
        }

        if let Some(rest) = strip_prefix(raw, b"%begin ") {
            let num = second_field_u64(rest);
            self.block = Some((num, Vec::new()));
            return ControlEvent::Ignored;
        }

        if let Some(rest) = strip_prefix(raw, b"%output ") {
            // "%output %<pane> <data>"
            let mut parts = rest.splitn(2, |&b| b == b' ');
            let pane = parts.next().unwrap_or_default();
            let data = parts.next().unwrap_or_default();
            return ControlEvent::Output {
                pane: PaneId(String::from_utf8_lossy(pane).into_owned()),
                bytes: vis_unescape(data),
            };
        }

        if let Some(rest) = strip_prefix(raw, b"%layout-change ") {
            // "%layout-change <window-id> <layout> [<visible-layout> <flags>]"
            let text = String::from_utf8_lossy(rest);
            let mut fields = text.split_whitespace();
            let window_id = fields.next().unwrap_or_default().to_string();
            let layout = fields.next().unwrap_or_default().to_string();
            return ControlEvent::LayoutChange { window_id, layout };
        }

        if let Some(rest) = strip_prefix(raw, b"%session-changed ") {
            let text = String::from_utf8_lossy(rest);
            let mut fields = text.splitn(2, ' ');
            return ControlEvent::SessionChanged {
                id: fields.next().unwrap_or_default().to_string(),
                name: fields.next().unwrap_or_default().to_string(),
            };
        }

        if let Some(rest) = strip_prefix(raw, b"%pane-mode-changed ") {
            return ControlEvent::PaneModeChanged {
                pane: PaneId(String::from_utf8_lossy(rest).trim().to_string()),
            };
        }

        if raw == b"%exit" || raw.starts_with(b"%exit ") {
            let reason = raw
                .get(6..)
                .map(|r| String::from_utf8_lossy(r).trim().to_string())
                .filter(|r| !r.is_empty());
            return ControlEvent::Exit { reason };
        }

        const WINDOW_EVENTS: &[&[u8]] = &[
            b"%window-add",
            b"%window-close",
            b"%unlinked-window-add",
            b"%unlinked-window-close",
            b"%window-renamed",
            b"%window-pane-changed",
            b"%session-window-changed",
        ];
        if WINDOW_EVENTS
            .iter()
            .any(|p| raw == *p || raw.starts_with(&[p, &b" "[..]].concat()))
        {
            return ControlEvent::WindowsChanged;
        }

        if raw.starts_with(b"%") {
            tracing::trace!(line = %String::from_utf8_lossy(raw), "unhandled control-mode notification");
        }
        ControlEvent::Ignored
    }

    fn finish_block(&mut self, rest: &[u8], ok: bool) -> ControlEvent {
        let (num, output) = self.block.take().unwrap_or((0, Vec::new()));
        // %end carries "<time> <num> <flags>"; verify the number matches when
        // parseable, but don't drop output on mismatch — tmux won't nest.
        let end_num = second_field_u64(rest);
        if end_num != num {
            tracing::trace!(begin = num, end = end_num, "control block number mismatch");
        }
        ControlEvent::CommandDone { num, ok, output }
    }
}

fn strip_prefix<'a>(raw: &'a [u8], prefix: &[u8]) -> Option<&'a [u8]> {
    raw.strip_prefix(prefix)
}

/// `%begin <time> <num> <flags>` → the `<num>` field.
fn second_field_u64(rest: &[u8]) -> u64 {
    let text = String::from_utf8_lossy(rest);
    text.split_whitespace()
        .nth(1)
        .and_then(|f| f.parse().ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(parser: &mut ControlParser, line: &str) -> ControlEvent {
        parser.feed_line(line.as_bytes())
    }

    #[test]
    fn output_is_unescaped() {
        let mut p = ControlParser::new();
        let ev = feed(&mut p, r"%output %3 hello\033[31mred\033[0m\015\012");
        match ev {
            ControlEvent::Output { pane, bytes } => {
                assert_eq!(pane.as_str(), "%3");
                assert_eq!(bytes, b"hello\x1b[31mred\x1b[0m\r\n");
            }
            other => panic!("expected Output, got {other:?}"),
        }
    }

    #[test]
    fn output_preserves_spaces_and_backslashes() {
        let mut p = ControlParser::new();
        let ev = feed(&mut p, r"%output %0 a b \\ c");
        match ev {
            ControlEvent::Output { bytes, .. } => assert_eq!(bytes, b"a b \\ c"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn command_block_collects_output() {
        let mut p = ControlParser::new();
        assert_eq!(
            feed(&mut p, "%begin 1751970000 12 1"),
            ControlEvent::Ignored
        );
        assert_eq!(feed(&mut p, "line one"), ControlEvent::Ignored);
        assert_eq!(feed(&mut p, "line two"), ControlEvent::Ignored);
        let ev = feed(&mut p, "%end 1751970000 12 1");
        assert_eq!(
            ev,
            ControlEvent::CommandDone {
                num: 12,
                ok: true,
                output: vec!["line one".into(), "line two".into()],
            }
        );
    }

    #[test]
    fn error_block_flags_not_ok() {
        let mut p = ControlParser::new();
        feed(&mut p, "%begin 1 7 1");
        feed(&mut p, "bad command");
        match feed(&mut p, "%error 1 7 1") {
            ControlEvent::CommandDone { ok, num, .. } => {
                assert!(!ok);
                assert_eq!(num, 7);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn notifications_inside_block_are_output_text() {
        // tmux never interleaves notifications inside a block; a %-looking
        // line of command output must be treated as text.
        let mut p = ControlParser::new();
        feed(&mut p, "%begin 1 2 1");
        feed(&mut p, "%output %9 fake");
        match feed(&mut p, "%end 1 2 1") {
            ControlEvent::CommandDone { output, .. } => {
                assert_eq!(output, vec!["%output %9 fake".to_string()]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn layout_change_and_exit() {
        let mut p = ControlParser::new();
        match feed(
            &mut p,
            "%layout-change @1 bd5b,80x24,0,0,3 bd5b,80x24,0,0,3 *",
        ) {
            ControlEvent::LayoutChange { window_id, layout } => {
                assert_eq!(window_id, "@1");
                assert_eq!(layout, "bd5b,80x24,0,0,3");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(feed(&mut p, "%exit"), ControlEvent::Exit { reason: None });
        assert_eq!(
            feed(&mut p, "%exit detached"),
            ControlEvent::Exit {
                reason: Some("detached".into())
            }
        );
    }

    #[test]
    fn window_events_collapse_to_windows_changed() {
        let mut p = ControlParser::new();
        for line in [
            "%window-add @5",
            "%window-close @5",
            "%window-renamed @2 build",
            "%unlinked-window-add @9",
        ] {
            assert_eq!(feed(&mut p, line), ControlEvent::WindowsChanged, "{line}");
        }
    }

    #[test]
    fn crlf_line_endings_tolerated() {
        let mut p = ControlParser::new();
        match p.feed_line(b"%output %1 data\r") {
            ControlEvent::Output { bytes, .. } => assert_eq!(bytes, b"data"),
            other => panic!("{other:?}"),
        }
    }
}
