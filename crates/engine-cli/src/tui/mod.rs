//! TUI application state (Phase 2). Kept free of terminal I/O so the whole
//! state machine is drivable from tests: `handle_key` + `tick` mutate state,
//! `ui::draw` renders it. The runner in `run.rs` owns the real terminal.

pub mod run;
pub mod ui;

use anyhow::Result;
use engine::{Button, Engine, EngineEvent, EventStream, GridSnapshot, PaneInfo};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::broadcast::error::TryRecvError;

/// Snapshot poll cadence while a pane is focused (DESIGN.md §4.1).
pub const POLL_INTERVAL_MS: u64 = 300;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    List,
    Pane,
}

pub struct App {
    engine: Engine,
    pub session: String,
    pub view: View,
    pub panes: Vec<PaneInfo>,
    pub selected: usize,
    pub grid: Option<GridSnapshot>,
    /// Quick-action buttons (generic set now; adapter-driven in Phase 4).
    pub buttons: Vec<Button>,
    pub input: String,
    /// Lines scrolled up from the live tail.
    pub scroll_offset: u32,
    pub status: String,
    pub should_quit: bool,
    /// Grid viewport (cols, rows) recorded by the last draw; drives paging
    /// and the fit-to-view action.
    pub grid_viewport: (u16, u16),
    /// Set when the next loop iteration should tick immediately.
    pub dirty: bool,
    /// Engine event stream (Grid/Panes/Connected/… — Phase 3 streaming).
    events: EventStream,
    /// True once attach() succeeded for the focused pane: grids arrive as
    /// events and snapshot polling stops (except in scrollback).
    pub streaming: bool,
    /// Last client size sent to the streamer, to reflow on viewport change.
    last_sent_size: Option<(u16, u16)>,
}

fn default_buttons() -> Vec<Button> {
    [
        ("Yes", "y"),
        ("No", "n"),
        ("Enter", "<Enter>"),
        ("Ctrl-C", "<C-c>"),
    ]
    .into_iter()
    .map(|(label, keys)| Button {
        label: label.to_string(),
        keys: keys.to_string(),
    })
    .collect()
}

impl App {
    pub fn new(engine: Engine, session: String) -> Self {
        let events = engine.subscribe();
        App {
            engine,
            session,
            view: View::List,
            panes: Vec::new(),
            selected: 0,
            grid: None,
            buttons: default_buttons(),
            input: String::new(),
            scroll_offset: 0,
            status: String::new(),
            should_quit: false,
            grid_viewport: (80, 24),
            dirty: true,
            events,
            streaming: false,
            last_sent_size: None,
        }
    }

    pub fn selected_pane(&self) -> Option<&PaneInfo> {
        self.panes.get(self.selected)
    }

    pub async fn refresh_panes(&mut self) {
        match self.engine.list_panes(&self.session).await {
            Ok(panes) => {
                self.panes = panes;
                if self.selected >= self.panes.len() {
                    self.selected = self.panes.len().saturating_sub(1);
                }
                self.status.clear();
            }
            Err(e) => self.status = format!("list panes: {e}"),
        }
    }

    /// Drain pending engine events into app state.
    fn drain_events(&mut self) {
        loop {
            match self.events.try_recv() {
                Ok(ev) => self.apply_event(ev),
                Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
                Err(TryRecvError::Lagged(_)) => continue,
            }
        }
    }

    fn apply_event(&mut self, ev: EngineEvent) {
        match ev {
            EngineEvent::Grid { pane, snapshot, .. } => {
                // Live tail only: in scrollback the poll path owns the grid.
                if self.view == View::Pane
                    && self.scroll_offset == 0
                    && self.selected_pane().is_some_and(|p| p.id == pane)
                {
                    self.grid = Some(snapshot);
                }
            }
            EngineEvent::Panes { session, panes } => {
                if session == self.session {
                    let keep = self.selected_pane().map(|p| p.id.clone());
                    self.panes = panes;
                    if let Some(id) = keep {
                        if let Some(i) = self.panes.iter().position(|p| p.id == id) {
                            self.selected = i;
                        }
                    }
                    self.selected = self.selected.min(self.panes.len().saturating_sub(1));
                }
            }
            EngineEvent::Reconnecting => self.status = "reconnecting…".into(),
            EngineEvent::Connected => self.status.clear(),
            EngineEvent::Error(e) => {
                self.status = format!("stream: {e}");
                self.streaming = false; // fall back to polling
            }
            _ => {}
        }
    }

    /// Periodic work: drain events, poll snapshots where streaming doesn't
    /// cover us, and push viewport changes to the streamer for reflow.
    pub async fn tick(&mut self) {
        self.dirty = false;
        self.drain_events();
        match self.view {
            View::List => self.refresh_panes().await,
            View::Pane => {
                let Some(pane) = self.selected_pane().map(|p| p.id.clone()) else {
                    self.view = View::List;
                    return;
                };
                // Rotation/resize: retarget the tmux client size (§4.3).
                if self.streaming && self.last_sent_size != Some(self.grid_viewport) {
                    let (cols, rows) = self.grid_viewport;
                    if self.engine.resize(&pane, cols, rows).await.is_ok() {
                        self.last_sent_size = Some(self.grid_viewport);
                    }
                }
                if self.streaming && self.scroll_offset == 0 {
                    return; // Grid events own the live tail
                }
                match self.engine.snapshot(&pane, self.scroll_offset).await {
                    Ok(grid) => {
                        // Clamp scroll to the history that actually exists.
                        let viewport_rows = self.grid_viewport.1 as u32;
                        let above = (grid.rows as u32).saturating_sub(viewport_rows.max(1));
                        self.scroll_offset = self.scroll_offset.min(above);
                        self.grid = Some(grid);
                        self.status.clear();
                    }
                    Err(e) => self.status = format!("snapshot: {e}"),
                }
            }
        }
    }

    async fn send_to_pane(&mut self, keys: &str, describe: &str) {
        let Some(pane) = self.selected_pane().map(|p| p.id.clone()) else {
            return;
        };
        match self.engine.send_key_string(&pane, keys).await {
            Ok(()) => {
                self.status = format!("sent {describe}");
                self.dirty = true;
            }
            Err(e) => self.status = format!("send: {e}"),
        }
    }

    pub async fn press_button(&mut self, index: usize) {
        if let Some(b) = self.buttons.get(index).cloned() {
            self.send_to_pane(&b.keys, &b.label).await;
        }
    }

    pub async fn handle_key(&mut self, key: KeyEvent) {
        match self.view {
            View::List => self.handle_key_list(key).await,
            View::Pane => self.handle_key_pane(key).await,
        }
    }

    async fn handle_key_list(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.selected + 1 < self.panes.len() {
                    self.selected += 1;
                }
            }
            KeyCode::Char('r') => self.refresh_panes().await,
            KeyCode::Enter | KeyCode::Char('l') => {
                if let Some(pane) = self.selected_pane().map(|p| p.id.clone()) {
                    self.view = View::Pane;
                    self.grid = None;
                    self.scroll_offset = 0;
                    self.input.clear();
                    self.dirty = true;
                    // Stream if we can; otherwise the tick() poll path covers us.
                    match self.engine.attach(&pane, self.grid_viewport).await {
                        Ok(()) => {
                            self.streaming = true;
                            self.last_sent_size = Some(self.grid_viewport);
                        }
                        Err(e) => {
                            self.streaming = false;
                            self.status = format!("streaming unavailable ({e}); polling");
                        }
                    }
                }
            }
            _ => {}
        }
    }

    async fn handle_key_pane(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                if self.input.is_empty() {
                    if let Some(pane) = self.selected_pane().map(|p| p.id.clone()) {
                        let _ = self.engine.detach(&pane).await;
                    }
                    self.streaming = false;
                    self.view = View::List;
                    self.dirty = true;
                } else {
                    self.input.clear();
                }
            }
            KeyCode::Char('q') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true
            }
            // Button row: F2..F5 (and Ctrl-y / Ctrl-n aliases for terminals
            // that swallow function keys).
            KeyCode::F(n @ 2..=5) => self.press_button(n as usize - 2).await,
            KeyCode::Char('y') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.press_button(0).await
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.press_button(1).await
            }
            // Fit the remote window to our viewport (explicit, so we never
            // fight a desktop client for sizing unless asked).
            KeyCode::F(8) => {
                let (cols, rows) = self.grid_viewport;
                let Some(pane) = self.selected_pane().map(|p| p.id.clone()) else {
                    return;
                };
                match self.engine.resize(&pane, cols.max(20), rows.max(5)).await {
                    Ok(()) => {
                        self.status = format!("resized to {cols}x{rows}");
                        self.dirty = true;
                    }
                    Err(e) => self.status = format!("resize: {e}"),
                }
            }
            KeyCode::PageUp => {
                let page = (self.grid_viewport.1 as u32).saturating_sub(1).max(1);
                self.scroll_offset = (self.scroll_offset + page).min(5_000);
                self.dirty = true;
            }
            KeyCode::PageDown => {
                let page = (self.grid_viewport.1 as u32).saturating_sub(1).max(1);
                self.scroll_offset = self.scroll_offset.saturating_sub(page);
                self.dirty = true;
            }
            KeyCode::End => {
                self.scroll_offset = 0;
                self.dirty = true;
            }
            KeyCode::Enter => {
                let text = std::mem::take(&mut self.input);
                if text.is_empty() {
                    self.send_to_pane("<Enter>", "Enter").await;
                } else {
                    // Literal text, then Enter. `<` in user text must stay
                    // literal, so build inputs directly instead of re-parsing.
                    let Some(pane) = self.selected_pane().map(|p| p.id.clone()) else {
                        return;
                    };
                    let inputs = [
                        engine::KeyInput::Text(text.clone()),
                        engine::KeyInput::Named(vec!["Enter".into()]),
                    ];
                    match self.engine.send_keys(&pane, &inputs).await {
                        Ok(()) => {
                            self.status = format!("sent text ({} chars)", text.chars().count());
                            self.dirty = true;
                        }
                        Err(e) => self.status = format!("send: {e}"),
                    }
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.input.push(c);
            }
            _ => {}
        }
    }

    /// Row window of the current grid for a `viewport_rows`-tall view,
    /// honoring the scroll offset: `(start, end)` into grid rows.
    pub fn visible_row_range(&self, viewport_rows: u16) -> (u16, u16) {
        let Some(grid) = &self.grid else {
            return (0, 0);
        };
        let above = grid.rows.saturating_sub(viewport_rows.max(1));
        let clamped = (self.scroll_offset as u16).min(above);
        let end = grid.rows - clamped;
        let start = end.saturating_sub(viewport_rows.max(1));
        (start, end)
    }
}

/// Convenience used by main.rs.
pub async fn run_tui(engine: Engine, session: String) -> Result<()> {
    run::run(App::new(engine, session)).await
}
