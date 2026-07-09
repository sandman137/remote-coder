//! Engine event model (DESIGN.md §7.2): a broadcast bus every UI subscribes
//! to. Snapshot mode (Phase 1/2) is pull-based; streaming (Phase 3) and
//! attention (Phase 4) push through here.

use std::collections::HashMap;

use crate::grid::GridSnapshot;
use crate::tmux::{PaneId, PaneInfo, SessionInfo};

/// Quick-action button surfaced by an adapter ("Yes" → keys "1").
/// `keys` uses the `<Name>` convention of [`crate::tmux::keys::parse_key_string`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Button {
    pub label: String,
    pub keys: String,
}

/// What kind of input an agent appears to be waiting for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptKind {
    YesNo,
    Menu,
    FreeText,
    Unknown,
}

#[derive(Debug, Clone)]
pub enum EngineEvent {
    Sessions(Vec<SessionInfo>),
    Panes {
        session: String,
        panes: Vec<PaneInfo>,
    },
    Grid {
        pane: PaneId,
        snapshot: GridSnapshot,
        dirty_rows: Vec<u16>,
    },
    Attention {
        pane: PaneId,
        agent: String,
        kind: PromptKind,
        buttons: Vec<Button>,
    },
    /// A previously-waiting pane no longer shows its prompt.
    AttentionCleared {
        pane: PaneId,
    },
    Metadata {
        pane: PaneId,
        fields: HashMap<String, String>,
    },
    Exited {
        pane: PaneId,
        status: Option<i32>,
    },
    Reconnecting,
    Connected,
    Error(String),
}

pub type EventStream = tokio::sync::broadcast::Receiver<EngineEvent>;

/// Fan-out bus. Slow subscribers lose oldest events (broadcast semantics) —
/// acceptable: grids are snapshots, and list events are re-fetchable.
#[derive(Debug, Clone)]
pub struct EventBus {
    tx: tokio::sync::broadcast::Sender<EngineEvent>,
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _) = tokio::sync::broadcast::channel(256);
        EventBus { tx }
    }

    pub fn subscribe(&self) -> EventStream {
        self.tx.subscribe()
    }

    pub fn emit(&self, event: EngineEvent) {
        // No subscribers is fine (headless CLI).
        let _ = self.tx.send(event);
    }
}
