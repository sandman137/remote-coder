//! tmux protocol layer: command builders + stable `-F` enumeration parsers
//! (DESIGN.md §4.4). Commands are built as argv arrays — never shell strings.

pub mod keys;

use crate::error::EngineError;

/// Field separator embedded in `-F` format strings. U+001F (unit separator)
/// can't be typed into a session/window/pane title by accident, unlike tabs.
/// (tmux does not interpret `\t` inside format strings, so we embed the
/// separator byte itself.)
pub const SEP: char = '\u{1f}';

/// Stable pane identifier (`%N`, unique per tmux server, survives renames).
/// Also accepts any tmux target string (`agents:0.0`) — it is passed to `-t`
/// verbatim, so both forms work anywhere a `PaneId` is taken.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PaneId(pub String);

impl PaneId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for PaneId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl From<&str> for PaneId {
    fn from(s: &str) -> Self {
        PaneId(s.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub name: String,
    pub windows: u32,
    /// Number of clients currently attached.
    pub attached: u32,
    /// Creation time, unix epoch seconds.
    pub created: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    pub session: String,
    pub window_index: u32,
    pub window_name: String,
    pub window_active: bool,
    pub id: PaneId,
    pub pane_index: u32,
    pub title: String,
    /// Foreground command in the pane (`claude`, `bash`, …) — adapter
    /// auto-detection keys off this.
    pub current_command: String,
    pub active: bool,
    pub width: u16,
    pub height: u16,
}

/// Size + cursor of a single pane, fetched alongside snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PaneGeometry {
    pub width: u16,
    pub height: u16,
    /// Cursor position, 0-based (col, row) within the visible area.
    pub cursor: (u16, u16),
}

fn session_format() -> String {
    [
        "#{session_name}",
        "#{session_windows}",
        "#{session_attached}",
        "#{session_created}",
    ]
    .join(&SEP.to_string())
}

fn pane_format() -> String {
    [
        "#{session_name}",
        "#{window_index}",
        "#{window_name}",
        "#{window_active}",
        "#{pane_id}",
        "#{pane_index}",
        "#{pane_title}",
        "#{pane_current_command}",
        "#{pane_active}",
        "#{pane_width}",
        "#{pane_height}",
    ]
    .join(&SEP.to_string())
}

fn geometry_format() -> String {
    [
        "#{pane_width}",
        "#{pane_height}",
        "#{cursor_x}",
        "#{cursor_y}",
    ]
    .join(&SEP.to_string())
}

/// Argv builders for every tmux command the engine issues. Centralized so the
/// broker whitelist (Phase 6) and the engine can never drift apart silently.
pub mod cmd {
    use super::*;

    pub fn list_sessions() -> Vec<String> {
        vec!["list-sessions".into(), "-F".into(), session_format()]
    }

    /// All panes of one session (`-t <session>`); `session = None` lists all.
    pub fn list_panes(session: Option<&str>) -> Vec<String> {
        let mut argv: Vec<String> = vec!["list-panes".into()];
        match session {
            Some(s) => {
                argv.push("-s".into()); // all panes in session
                argv.push("-t".into());
                // `=` prefix: exact match, no name-prefix guessing.
                argv.push(format!("={s}"));
            }
            None => argv.push("-a".into()),
        }
        argv.push("-F".into());
        argv.push(pane_format());
        argv
    }

    pub fn display_geometry(pane: &PaneId) -> Vec<String> {
        vec![
            "display-message".into(),
            "-p".into(),
            "-t".into(),
            pane.0.clone(),
            geometry_format(),
        ]
    }

    /// Visible screen (+ optional scrollback lines above it), SGR escapes
    /// included (`-e`), resolved into rows by tmux — no VT parsing needed.
    pub fn capture_pane(pane: &PaneId, scrollback: u32) -> Vec<String> {
        let mut argv: Vec<String> = vec![
            "capture-pane".into(),
            "-p".into(),
            "-e".into(),
            "-t".into(),
            pane.0.clone(),
        ];
        if scrollback > 0 {
            argv.push("-S".into());
            argv.push(format!("-{scrollback}"));
        }
        argv
    }

    /// Literal text: `send-keys -l -- <text>`.
    pub fn send_literal(pane: &PaneId, text: &str) -> Vec<String> {
        vec![
            "send-keys".into(),
            "-t".into(),
            pane.0.clone(),
            "-l".into(),
            "--".into(),
            text.into(),
        ]
    }

    /// Named keys: `send-keys -- Enter C-c …`.
    pub fn send_named(pane: &PaneId, keys: &[String]) -> Vec<String> {
        let mut argv: Vec<String> =
            vec!["send-keys".into(), "-t".into(), pane.0.clone(), "--".into()];
        argv.extend(keys.iter().cloned());
        argv
    }

    pub fn resize_window(session: &str, cols: u16, rows: u16) -> Vec<String> {
        vec![
            "resize-window".into(),
            "-t".into(),
            format!("={session}"),
            "-x".into(),
            cols.to_string(),
            "-y".into(),
            rows.to_string(),
        ]
    }

    /// Launch an agent in a fresh window of `session`; prints the new pane id.
    pub fn new_window(
        session: &str,
        name: &str,
        cwd: Option<&str>,
        shell_cmd: &str,
    ) -> Vec<String> {
        let mut argv: Vec<String> = vec![
            "new-window".into(),
            "-t".into(),
            format!("={session}"),
            "-n".into(),
            name.into(),
            "-P".into(),
            "-F".into(),
            "#{pane_id}".into(),
        ];
        if let Some(dir) = cwd {
            argv.push("-c".into());
            argv.push(dir.into());
        }
        argv.push(shell_cmd.into());
        argv
    }
}

/// Decode tmux's vis(3)-style output escaping: `\\` → `\`, `\ooo` (three
/// octal digits) → that byte. tmux applies this to *all* its text output —
/// `-F` enumeration lines here, and `%output` payloads in control mode
/// (Phase 3), where missing this is the classic correctness bug
/// (DESIGN.md §4.2). Invalid escapes pass through untouched.
pub fn vis_unescape(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        let b = input[i];
        if b != b'\\' {
            out.push(b);
            i += 1;
            continue;
        }
        match input.get(i + 1) {
            Some(b'\\') => {
                out.push(b'\\');
                i += 2;
            }
            Some(&d0 @ b'0'..=b'7') => {
                // Expect exactly three octal digits (tmux always emits three).
                if let (Some(&d1 @ b'0'..=b'7'), Some(&d2 @ b'0'..=b'7')) =
                    (input.get(i + 2), input.get(i + 3))
                {
                    let val =
                        ((d0 - b'0') as u16) * 64 + ((d1 - b'0') as u16) * 8 + (d2 - b'0') as u16;
                    out.push(val as u8);
                    i += 4;
                } else {
                    out.push(b);
                    i += 1;
                }
            }
            _ => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Unescape one `-F` output line, then split into fields on the U+001F
/// separator we embedded in the format string.
fn split_fields(line: &str) -> Vec<String> {
    let decoded = vis_unescape(line.as_bytes());
    String::from_utf8_lossy(&decoded)
        .split(SEP)
        .map(str::to_string)
        .collect()
}

fn parse_err(what: &str, line: &str) -> EngineError {
    EngineError::Parse(format!("bad {what} line: {line:?}"))
}

pub fn parse_sessions(stdout: &[u8]) -> Result<Vec<SessionInfo>, EngineError> {
    let text = String::from_utf8_lossy(stdout);
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let f = split_fields(line);
        if f.len() != 4 {
            return Err(parse_err("session", line));
        }
        out.push(SessionInfo {
            name: f[0].to_string(),
            windows: f[1].parse().map_err(|_| parse_err("session", line))?,
            attached: f[2].parse().map_err(|_| parse_err("session", line))?,
            created: f[3].parse().map_err(|_| parse_err("session", line))?,
        });
    }
    Ok(out)
}

pub fn parse_panes(stdout: &[u8]) -> Result<Vec<PaneInfo>, EngineError> {
    let text = String::from_utf8_lossy(stdout);
    let mut out = Vec::new();
    for line in text.lines().filter(|l| !l.is_empty()) {
        let f = split_fields(line);
        if f.len() != 11 {
            return Err(parse_err("pane", line));
        }
        out.push(PaneInfo {
            session: f[0].to_string(),
            window_index: f[1].parse().map_err(|_| parse_err("pane", line))?,
            window_name: f[2].to_string(),
            window_active: f[3] == "1",
            id: PaneId(f[4].to_string()),
            pane_index: f[5].parse().map_err(|_| parse_err("pane", line))?,
            title: f[6].to_string(),
            current_command: f[7].to_string(),
            active: f[8] == "1",
            width: f[9].parse().map_err(|_| parse_err("pane", line))?,
            height: f[10].parse().map_err(|_| parse_err("pane", line))?,
        });
    }
    Ok(out)
}

pub fn parse_geometry(stdout: &[u8]) -> Result<PaneGeometry, EngineError> {
    let text = String::from_utf8_lossy(stdout);
    let line = text
        .lines()
        .find(|l| !l.is_empty())
        .ok_or_else(|| parse_err("geometry", &text))?;
    let f = split_fields(line);
    if f.len() != 4 {
        return Err(parse_err("geometry", line));
    }
    let num = |s: &str| -> Result<u16, EngineError> {
        s.parse().map_err(|_| parse_err("geometry", line))
    };
    Ok(PaneGeometry {
        width: num(&f[0])?,
        height: num(&f[1])?,
        cursor: (num(&f[2])?, num(&f[3])?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn j(fields: &[&str]) -> String {
        fields.join(&SEP.to_string())
    }

    #[test]
    fn parses_sessions() {
        let raw = format!(
            "{}\n{}\n",
            j(&["agents", "3", "1", "1751970000"]),
            j(&["dev", "1", "0", "1751970001"])
        );
        let s = parse_sessions(raw.as_bytes()).unwrap();
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].name, "agents");
        assert_eq!(s[0].windows, 3);
        assert_eq!(s[0].attached, 1);
        assert_eq!(s[1].name, "dev");
    }

    #[test]
    fn parses_panes_with_awkward_titles() {
        // Titles may contain spaces, colons, even tabs — the U+001F separator
        // keeps parsing unambiguous.
        let raw = j(&[
            "agents",
            "0",
            "yn",
            "1",
            "%5",
            "0",
            "a: weird\ttitle",
            "claude",
            "1",
            "100",
            "30",
        ]);
        let p = parse_panes(raw.as_bytes()).unwrap();
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].id, PaneId("%5".into()));
        assert_eq!(p[0].title, "a: weird\ttitle");
        assert_eq!(p[0].current_command, "claude");
        assert!(p[0].active && p[0].window_active);
        assert_eq!((p[0].width, p[0].height), (100, 30));
    }

    #[test]
    fn rejects_malformed_lines() {
        assert!(parse_panes(b"not a pane line").is_err());
        assert!(parse_sessions(b"only\x1fthree\x1ffields").is_err());
    }

    #[test]
    fn vis_unescape_decodes_octal_and_backslash() {
        assert_eq!(vis_unescape(b"a\\037b"), b"a\x1fb");
        assert_eq!(vis_unescape(b"back\\\\slash"), b"back\\slash");
        assert_eq!(vis_unescape(b"\\033[31m"), b"\x1b[31m");
        // Invalid/truncated escapes pass through.
        assert_eq!(vis_unescape(b"end\\"), b"end\\");
        assert_eq!(vis_unescape(b"\\9x"), b"\\9x");
        assert_eq!(vis_unescape(b"\\03"), b"\\03");
        // Multi-byte UTF-8 escaped byte-by-byte reassembles.
        assert_eq!(vis_unescape(b"\\303\\251"), "é".as_bytes());
    }

    #[test]
    fn parses_real_tmux_escaped_output() {
        // What tmux actually emits: the embedded U+001F arrives as `\037`.
        let raw = b"agents\\0373\\0371\\0371751970000\n";
        let s = parse_sessions(raw).unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].name, "agents");
        assert_eq!(s[0].windows, 3);
        assert_eq!(s[0].attached, 1);
        assert_eq!(s[0].created, 1751970000);
    }

    #[test]
    fn parses_geometry() {
        let raw = j(&["80", "24", "10", "23"]);
        let g = parse_geometry(raw.as_bytes()).unwrap();
        assert_eq!((g.width, g.height), (80, 24));
        assert_eq!(g.cursor, (10, 23));
    }

    #[test]
    fn capture_argv_includes_scrollback_only_when_asked() {
        let pane = PaneId("%1".into());
        assert!(!cmd::capture_pane(&pane, 0).contains(&"-S".to_string()));
        let with = cmd::capture_pane(&pane, 500);
        let i = with.iter().position(|a| a == "-S").unwrap();
        assert_eq!(with[i + 1], "-500");
    }

    #[test]
    fn send_literal_guards_dash_text() {
        let argv = cmd::send_literal(&PaneId("%1".into()), "-n");
        // `--` must precede the text so "-n" is not parsed as a flag.
        let dd = argv.iter().position(|a| a == "--").unwrap();
        assert_eq!(argv[dd + 1], "-n");
        assert!(argv.contains(&"-l".to_string()));
    }
}
