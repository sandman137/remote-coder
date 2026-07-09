//! FFI-shaped types (DESIGN.md §7.3): concrete and flat — no generics, no
//! trait objects, no lifetimes. `char` becomes a one-char `String` (UniFFI
//! has no char type); the attr bitfield rides as a `u8`.

use engine::{
    Button, Cell, CellAttrs, Color, EngineEvent, GridSnapshot, PaneInfo, PromptKind, SessionInfo,
};

#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum ColorFfi {
    Default,
    Indexed { index: u8 },
    Rgb { r: u8, g: u8, b: u8 },
}

impl From<Color> for ColorFfi {
    fn from(c: Color) -> Self {
        match c {
            Color::Default => ColorFfi::Default,
            Color::Indexed(n) => ColorFfi::Indexed { index: n },
            Color::Rgb(r, g, b) => ColorFfi::Rgb { r, g, b },
        }
    }
}

#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct CellFfi {
    /// The character as a string (UniFFI has no `char`).
    pub ch: String,
    pub fg: ColorFfi,
    pub bg: ColorFfi,
    /// Attribute bitfield (see CellAttrs constants).
    pub attrs: u8,
    /// Second cell of a wide char — renderers skip it.
    pub wide_continuation: bool,
}

impl From<&Cell> for CellFfi {
    fn from(c: &Cell) -> Self {
        CellFfi {
            ch: c.ch.to_string(),
            fg: c.fg.into(),
            bg: c.bg.into(),
            attrs: c.attrs.bits(),
            wide_continuation: c.attrs.has(CellAttrs::WIDE_CONT),
        }
    }
}

#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct CursorFfi {
    pub col: u16,
    pub row: u16,
}

#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct GridSnapshotFfi {
    pub cols: u16,
    pub rows: u16,
    pub cells: Vec<CellFfi>,
    pub cursor: Option<CursorFfi>,
}

impl From<&GridSnapshot> for GridSnapshotFfi {
    fn from(g: &GridSnapshot) -> Self {
        GridSnapshotFfi {
            cols: g.cols,
            rows: g.rows,
            cells: g.cells.iter().map(CellFfi::from).collect(),
            cursor: g.cursor.map(|(col, row)| CursorFfi { col, row }),
        }
    }
}

impl GridSnapshotFfi {
    /// Plain text of a row (convenience for renderers that want a quick line).
    pub fn row_text(&self, row: u16) -> String {
        if row >= self.rows {
            return String::new();
        }
        let start = row as usize * self.cols as usize;
        let end = start + self.cols as usize;
        let mut s: String = self.cells[start..end.min(self.cells.len())]
            .iter()
            .filter(|c| !c.wide_continuation)
            .map(|c| c.ch.as_str())
            .collect();
        s.truncate(s.trim_end().len());
        s
    }
}

#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct SessionInfoFfi {
    pub name: String,
    pub windows: u32,
    pub attached: u32,
    pub created: i64,
}

impl From<&SessionInfo> for SessionInfoFfi {
    fn from(s: &SessionInfo) -> Self {
        SessionInfoFfi {
            name: s.name.clone(),
            windows: s.windows,
            attached: s.attached,
            created: s.created,
        }
    }
}

#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct PaneInfoFfi {
    pub session: String,
    pub window_index: u32,
    pub window_name: String,
    pub window_active: bool,
    pub id: String,
    pub pane_index: u32,
    pub title: String,
    pub current_command: String,
    pub active: bool,
    pub width: u16,
    pub height: u16,
}

impl From<&PaneInfo> for PaneInfoFfi {
    fn from(p: &PaneInfo) -> Self {
        PaneInfoFfi {
            session: p.session.clone(),
            window_index: p.window_index,
            window_name: p.window_name.clone(),
            window_active: p.window_active,
            id: p.id.0.clone(),
            pane_index: p.pane_index,
            title: p.title.clone(),
            current_command: p.current_command.clone(),
            active: p.active,
            width: p.width,
            height: p.height,
        }
    }
}

#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct ButtonFfi {
    pub label: String,
    pub keys: String,
}

impl From<&Button> for ButtonFfi {
    fn from(b: &Button) -> Self {
        ButtonFfi {
            label: b.label.clone(),
            keys: b.keys.clone(),
        }
    }
}

#[derive(uniffi::Enum, Debug, Clone, Copy, PartialEq)]
pub enum PromptKindFfi {
    YesNo,
    Menu,
    FreeText,
    Unknown,
}

impl From<PromptKind> for PromptKindFfi {
    fn from(k: PromptKind) -> Self {
        match k {
            PromptKind::YesNo => PromptKindFfi::YesNo,
            PromptKind::Menu => PromptKindFfi::Menu,
            PromptKind::FreeText => PromptKindFfi::FreeText,
            PromptKind::Unknown => PromptKindFfi::Unknown,
        }
    }
}

/// A metadata key/value, flattened from the engine's map (UniFFI records
/// can't be keyed by dynamic maps in every language cleanly, and an ordered
/// list of pairs renders fine everywhere).
#[derive(uniffi::Record, Debug, Clone, PartialEq)]
pub struct MetaFieldFfi {
    pub field: String,
    pub value: String,
}

#[derive(uniffi::Enum, Debug, Clone, PartialEq)]
pub enum EngineEventFfi {
    Sessions {
        sessions: Vec<SessionInfoFfi>,
    },
    Panes {
        session: String,
        panes: Vec<PaneInfoFfi>,
    },
    Grid {
        pane: String,
        snapshot: GridSnapshotFfi,
        dirty_rows: Vec<u16>,
    },
    Attention {
        pane: String,
        agent: String,
        kind: PromptKindFfi,
        buttons: Vec<ButtonFfi>,
    },
    AttentionCleared {
        pane: String,
    },
    Metadata {
        pane: String,
        fields: Vec<MetaFieldFfi>,
    },
    Exited {
        pane: String,
        status: Option<i32>,
    },
    Reconnecting,
    Connected,
    Error {
        message: String,
    },
}

impl From<EngineEvent> for EngineEventFfi {
    fn from(e: EngineEvent) -> Self {
        match e {
            EngineEvent::Sessions(s) => EngineEventFfi::Sessions {
                sessions: s.iter().map(SessionInfoFfi::from).collect(),
            },
            EngineEvent::Panes { session, panes } => EngineEventFfi::Panes {
                session,
                panes: panes.iter().map(PaneInfoFfi::from).collect(),
            },
            EngineEvent::Grid {
                pane,
                snapshot,
                dirty_rows,
            } => EngineEventFfi::Grid {
                pane: pane.0,
                snapshot: GridSnapshotFfi::from(&snapshot),
                dirty_rows,
            },
            EngineEvent::Attention {
                pane,
                agent,
                kind,
                buttons,
            } => EngineEventFfi::Attention {
                pane: pane.0,
                agent,
                kind: kind.into(),
                buttons: buttons.iter().map(ButtonFfi::from).collect(),
            },
            EngineEvent::AttentionCleared { pane } => {
                EngineEventFfi::AttentionCleared { pane: pane.0 }
            }
            EngineEvent::Metadata { pane, fields } => {
                let mut fields: Vec<MetaFieldFfi> = fields
                    .into_iter()
                    .map(|(field, value)| MetaFieldFfi { field, value })
                    .collect();
                fields.sort_by(|a, b| a.field.cmp(&b.field));
                EngineEventFfi::Metadata {
                    pane: pane.0,
                    fields,
                }
            }
            EngineEvent::Exited { pane, status } => EngineEventFfi::Exited {
                pane: pane.0,
                status,
            },
            EngineEvent::Reconnecting => EngineEventFfi::Reconnecting,
            EngineEvent::Connected => EngineEventFfi::Connected,
            EngineEvent::Error(message) => EngineEventFfi::Error { message },
        }
    }
}

/// Connection config (flattened `engine::ConnConfig`).
#[derive(uniffi::Enum, Debug, Clone)]
pub enum ConnConfigFfi {
    Local {
        socket: Option<String>,
    },
    Ssh {
        host: String,
        port: u16,
        user: String,
        key_path: String,
        hostkey_fp: Option<String>,
    },
}

/// Cell attribute bit constants, re-exported for foreign renderers.
#[derive(uniffi::Record, Debug, Clone, Copy)]
pub struct CellAttrBits {
    pub bold: u8,
    pub dim: u8,
    pub italic: u8,
    pub underline: u8,
    pub reverse: u8,
    pub strike: u8,
    pub blink: u8,
}

#[uniffi::export]
pub fn cell_attr_bits() -> CellAttrBits {
    CellAttrBits {
        bold: CellAttrs::BOLD,
        dim: CellAttrs::DIM,
        italic: CellAttrs::ITALIC,
        underline: CellAttrs::UNDERLINE,
        reverse: CellAttrs::REVERSE,
        strike: CellAttrs::STRIKE,
        blink: CellAttrs::BLINK,
    }
}
