//! Session streamer (DESIGN.md §4.2): one task per attached session owning
//! the control-mode channel. It keeps a live [`VtScreen`] per pane (fed from
//! unescaped `%output`), reacts to layout changes by resizing + re-priming
//! from `capture-pane`, coalesces redraws, and emits `EngineEvent::Grid`
//! for watched panes. Reconnects with backoff when the channel drops (§13).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::error::{EngineError, TransportError};
use crate::event::{EngineEvent, EventBus};
use crate::grid::vt::VtScreen;
use crate::grid::GridSnapshot;
use crate::tmux::control::{ControlEvent, ControlParser};
use crate::tmux::layout::parse_layout;
use crate::tmux::{cmd, parse_geometry, parse_panes, PaneId};
use crate::transport::{ControlChannel, Transport};

const FLUSH_INTERVAL: Duration = Duration::from_millis(33);
const RECONNECT_ATTEMPTS: u32 = 5;
const RECONNECT_BASE_DELAY: Duration = Duration::from_millis(400);

#[derive(Debug, PartialEq, Eq)]
enum LineOutcome {
    Continue,
    Exit,
}

#[derive(Debug)]
pub(crate) enum StreamCmd {
    SetSize { cols: u16, rows: u16 },
    Watch(PaneId),
    Unwatch(PaneId),
}

pub(crate) struct StreamHandle {
    pub cmd_tx: mpsc::UnboundedSender<StreamCmd>,
    task: tokio::task::JoinHandle<()>,
}

impl StreamHandle {
    pub fn is_alive(&self) -> bool {
        !self.task.is_finished()
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Open the control channel (errors surface to the caller), then hand it to
/// the background task.
pub(crate) async fn spawn(
    transport: Arc<dyn Transport>,
    session: String,
    size: (u16, u16),
    bus: EventBus,
) -> Result<StreamHandle, EngineError> {
    let channel = transport.open_control(&session, size).await?;
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let mut streamer = Streamer {
        transport,
        session,
        bus,
        size,
        screens: HashMap::new(),
        prev: HashMap::new(),
        watched: HashSet::new(),
        dirty: HashSet::new(),
        parser: ControlParser::new(),
    };
    let task = tokio::spawn(async move {
        streamer.run(channel, cmd_rx).await;
    });
    Ok(StreamHandle { cmd_tx, task })
}

struct Streamer {
    transport: Arc<dyn Transport>,
    session: String,
    bus: EventBus,
    size: (u16, u16),
    screens: HashMap<String, VtScreen>,
    prev: HashMap<String, GridSnapshot>,
    watched: HashSet<String>,
    dirty: HashSet<String>,
    parser: ControlParser,
}

impl Streamer {
    async fn run(
        &mut self,
        mut channel: Box<dyn ControlChannel>,
        mut cmd_rx: mpsc::UnboundedReceiver<StreamCmd>,
    ) {
        if let Err(e) = self.start_channel(&mut channel).await {
            self.bus
                .emit(EngineEvent::Error(format!("stream start: {e}")));
            return;
        }
        self.bus.emit(EngineEvent::Connected);

        let mut flush = tokio::time::interval(FLUSH_INTERVAL);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            // A channel is dead on EOF/error — or on `%exit`: after a detach
            // the tmux -C process lingers reading stdin and never EOFs, so
            // the notification itself is the terminal signal.
            let mut dead = false;
            tokio::select! {
                cmd = cmd_rx.recv() => match cmd {
                    None => break, // engine dropped the handle
                    Some(cmd) => {
                        if let Err(e) = self.handle_cmd(&mut channel, cmd).await {
                            tracing::debug!(error = %e, "stream command failed");
                        }
                    }
                },
                line = channel.read_line() => match line {
                    Ok(Some(raw)) => dead = self.handle_line(&raw).await == LineOutcome::Exit,
                    Ok(None) | Err(_) => dead = true,
                },
                _ = flush.tick() => self.flush_dirty(),
            }

            if dead {
                match self.reconnect().await {
                    Some(new_channel) => {
                        channel = new_channel; // old client killed on drop
                        if let Err(e) = self.start_channel(&mut channel).await {
                            self.bus
                                .emit(EngineEvent::Error(format!("stream restart: {e}")));
                            break;
                        }
                        self.bus.emit(EngineEvent::Connected);
                    }
                    None => {
                        self.bus.emit(EngineEvent::Error(format!(
                            "control channel to session {} lost",
                            self.session
                        )));
                        break;
                    }
                }
            }
        }
    }

    /// Set client size and prime every pane's screen from capture-pane.
    async fn start_channel(
        &mut self,
        channel: &mut Box<dyn ControlChannel>,
    ) -> Result<(), EngineError> {
        let (cols, rows) = self.size;
        channel
            .write_line(&format!("refresh-client -C {cols}x{rows}"))
            .await?;

        self.screens.clear();
        let panes = parse_panes(
            &self
                .transport
                .exec(&cmd::list_panes(Some(&self.session)))
                .await?,
        )?;
        for pane in &panes {
            self.prime_pane(&pane.id, pane.width, pane.height).await;
            self.dirty.insert(pane.id.0.clone());
        }
        self.bus.emit(EngineEvent::Panes {
            session: self.session.clone(),
            panes,
        });
        Ok(())
    }

    async fn handle_cmd(
        &mut self,
        channel: &mut Box<dyn ControlChannel>,
        cmd: StreamCmd,
    ) -> Result<(), TransportError> {
        match cmd {
            StreamCmd::SetSize { cols, rows } => {
                self.size = (cols, rows);
                channel
                    .write_line(&format!("refresh-client -C {cols}x{rows}"))
                    .await?;
            }
            StreamCmd::Watch(pane) => {
                if self.watched.insert(pane.0.clone()) {
                    // Ensure a screen exists even before first output.
                    if !self.screens.contains_key(&pane.0) {
                        self.prime_pane_auto(&pane).await;
                    }
                    self.dirty.insert(pane.0.clone());
                    self.prev.remove(&pane.0); // force full emit
                    self.flush_dirty();
                }
            }
            StreamCmd::Unwatch(pane) => {
                self.watched.remove(&pane.0);
            }
        }
        Ok(())
    }

    async fn handle_line(&mut self, raw: &[u8]) -> LineOutcome {
        match self.parser.feed_line(raw) {
            ControlEvent::Output { pane, bytes } => {
                match self.screens.get_mut(&pane.0) {
                    Some(screen) => screen.feed(&bytes),
                    None => {
                        // First sight of this pane: prime, then feed.
                        self.prime_pane_auto(&pane).await;
                        if let Some(screen) = self.screens.get_mut(&pane.0) {
                            screen.feed(&bytes);
                        }
                    }
                }
                self.dirty.insert(pane.0.clone());
            }
            ControlEvent::LayoutChange { layout, .. } => {
                if let Ok(node) = parse_layout(&layout) {
                    for (pane, rect) in node.leaves() {
                        let resized = self
                            .screens
                            .get(&pane.0)
                            .is_some_and(|s| s.size() != (rect.width, rect.height));
                        if resized || !self.screens.contains_key(&pane.0) {
                            self.prime_pane(pane, rect.width, rect.height).await;
                            self.dirty.insert(pane.0.clone());
                        }
                    }
                }
                self.emit_panes().await;
            }
            ControlEvent::WindowsChanged => self.emit_panes().await,
            ControlEvent::Exit { reason } => {
                tracing::debug!(?reason, session = %self.session, "control client exiting");
                return LineOutcome::Exit;
            }
            ControlEvent::CommandDone { ok, output, .. } => {
                if !ok {
                    tracing::debug!(?output, "control-mode command failed");
                }
            }
            ControlEvent::SessionChanged { .. }
            | ControlEvent::PaneModeChanged { .. }
            | ControlEvent::Ignored => {}
        }
        LineOutcome::Continue
    }

    /// (Re)create a pane's screen at the given size and load current content.
    async fn prime_pane(&mut self, pane: &PaneId, cols: u16, rows: u16) {
        let mut screen = VtScreen::new(cols, rows);
        let capture = self.transport.exec(&cmd::capture_pane(pane, 0)).await;
        let geo = self.transport.exec(&cmd::display_geometry(pane)).await;
        if let (Ok(capture), Ok(geo_raw)) = (capture, geo) {
            let cursor = parse_geometry(&geo_raw).map(|g| g.cursor).unwrap_or((0, 0));
            screen.prime(&capture, cursor);
        }
        self.prev.remove(&pane.0);
        self.screens.insert(pane.0.clone(), screen);
    }

    /// Prime with size discovered from tmux (new pane mid-stream).
    async fn prime_pane_auto(&mut self, pane: &PaneId) {
        let geo = match self.transport.exec(&cmd::display_geometry(pane)).await {
            Ok(raw) => parse_geometry(&raw).ok(),
            Err(_) => None,
        };
        let (cols, rows) = geo.map(|g| (g.width, g.height)).unwrap_or((80, 24));
        self.prime_pane(pane, cols, rows).await;
    }

    async fn emit_panes(&mut self) {
        if let Ok(raw) = self
            .transport
            .exec(&cmd::list_panes(Some(&self.session)))
            .await
        {
            if let Ok(panes) = parse_panes(&raw) {
                self.bus.emit(EngineEvent::Panes {
                    session: self.session.clone(),
                    panes,
                });
            }
        }
    }

    /// Emit Grid events for watched panes whose screens changed.
    fn flush_dirty(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        for pane_id in std::mem::take(&mut self.dirty) {
            if !self.watched.contains(&pane_id) {
                continue;
            }
            let Some(screen) = self.screens.get(&pane_id) else {
                continue;
            };
            let snapshot = screen.snapshot();
            let dirty_rows = match self.prev.get(&pane_id) {
                Some(prev) => {
                    let rows = snapshot.dirty_rows(prev);
                    if rows.is_empty() {
                        continue; // no visible change
                    }
                    rows
                }
                None => (0..snapshot.rows).collect(),
            };
            self.prev.insert(pane_id.clone(), snapshot.clone());
            self.bus.emit(EngineEvent::Grid {
                pane: PaneId(pane_id),
                snapshot,
                dirty_rows,
            });
        }
    }

    async fn reconnect(&mut self) -> Option<Box<dyn ControlChannel>> {
        self.bus.emit(EngineEvent::Reconnecting);
        for attempt in 0..RECONNECT_ATTEMPTS {
            tokio::time::sleep(RECONNECT_BASE_DELAY * 2u32.saturating_pow(attempt)).await;
            match self.transport.open_control(&self.session, self.size).await {
                Ok(channel) => {
                    self.parser = ControlParser::new();
                    return Some(channel);
                }
                Err(e) => {
                    tracing::debug!(attempt, error = %e, "reconnect attempt failed");
                }
            }
        }
        None
    }
}
