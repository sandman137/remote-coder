//! The `Engine` facade (DESIGN.md §7.1) — the one public entrypoint UIs use.

use std::sync::Arc;

use crate::error::{EngineError, Result, TransportError};
use crate::event::{EventBus, EventStream};
use crate::grid::{sgr, GridSnapshot};
use crate::tmux::keys::{parse_key_string, KeyInput};
use crate::tmux::{
    cmd, parse_geometry, parse_panes, parse_sessions, PaneId, PaneInfo, SessionInfo,
};
use crate::transport::{LocalTransport, Transport};

/// How to reach the tmux server (DESIGN.md §7.1).
#[derive(Debug, Clone)]
pub enum ConnConfig {
    /// Same-host tmux. `socket` = `tmux -L <socket>` (tests use a private
    /// server); `None` = the default server.
    Local { socket: Option<String> },
    /// SSH via russh with host-key pinning — arrives in Phase 5.
    Ssh {
        host: String,
        port: u16,
        user: String,
        key_path: Option<String>,
        hostkey_fp: Option<String>,
    },
}

pub struct Engine {
    transport: Arc<dyn Transport>,
    events: EventBus,
}

impl Engine {
    pub async fn connect(cfg: ConnConfig) -> Result<Engine> {
        let transport: Arc<dyn Transport> = match cfg {
            ConnConfig::Local { socket } => Arc::new(LocalTransport::new(socket)),
            ConnConfig::Ssh { .. } => {
                return Err(TransportError::Unsupported("SshTransport arrives in Phase 5").into())
            }
        };
        Ok(Engine {
            transport,
            events: EventBus::new(),
        })
    }

    pub fn subscribe(&self) -> EventStream {
        self.events.subscribe()
    }

    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        match self.transport.exec(&cmd::list_sessions()).await {
            Ok(out) => parse_sessions(&out),
            // A dev host with no tmux server yet is "no sessions", not an
            // error. tmux phrases it two ways depending on whether the socket
            // file exists ("no server running on …") or not ("error
            // connecting to … (No such file or directory)").
            Err(TransportError::Tmux { stderr, .. })
                if stderr.contains("no server running")
                    || stderr.contains("No such file or directory") =>
            {
                Ok(Vec::new())
            }
            Err(e) => Err(e.into()),
        }
    }

    /// Panes of one session (windows flattened — the UI groups by window).
    pub async fn list_panes(&self, session: &str) -> Result<Vec<PaneInfo>> {
        let out = self.transport.exec(&cmd::list_panes(Some(session))).await?;
        parse_panes(&out)
    }

    /// Render a pane via `capture-pane` (snapshot mode). `scrollback` = how
    /// many history lines above the visible screen to include (capped so
    /// unbounded history can never be streamed to a phone — DESIGN.md §13).
    pub async fn snapshot(&self, pane: &PaneId, scrollback: u32) -> Result<GridSnapshot> {
        const MAX_SCROLLBACK: u32 = 5_000;
        let scrollback = scrollback.min(MAX_SCROLLBACK);

        let geo_raw = self.transport.exec(&cmd::display_geometry(pane)).await?;
        let geo = parse_geometry(&geo_raw)?;

        let raw = self
            .transport
            .exec(&cmd::capture_pane(pane, scrollback))
            .await?;
        let mut grid = sgr::parse_capture(&raw, geo.width, geo.height);

        // capture output = [scrollback…][visible]; cursor coords are relative
        // to the visible area, so offset by whatever precedes it.
        let offset = grid.rows.saturating_sub(geo.height);
        let (cx, cy) = geo.cursor;
        let (col, row) = (cx.min(geo.width.saturating_sub(1)), offset + cy);
        if row < grid.rows {
            grid.cursor = Some((col, row));
        }
        Ok(grid)
    }

    pub async fn send_keys(&self, pane: &PaneId, inputs: &[KeyInput]) -> Result<()> {
        for input in inputs {
            let argv = match input {
                KeyInput::Text(t) => cmd::send_literal(pane, t),
                KeyInput::Named(keys) => {
                    if keys.is_empty() {
                        continue;
                    }
                    cmd::send_named(pane, keys)
                }
            };
            self.transport.exec(&argv).await?;
        }
        Ok(())
    }

    /// Convenience: send a `<Name>`-convention string ("y<Enter>").
    pub async fn send_key_string(&self, pane: &PaneId, keys: &str) -> Result<()> {
        self.send_keys(pane, &parse_key_string(keys)).await
    }

    /// Resize the window containing `pane` (snapshot-mode reflow; streaming
    /// mode gains `refresh-client -C` in Phase 3).
    pub async fn resize(&self, pane: &PaneId, cols: u16, rows: u16) -> Result<()> {
        let argv = vec![
            "resize-window".to_string(),
            "-t".to_string(),
            pane.as_str().to_string(),
            "-x".to_string(),
            cols.to_string(),
            "-y".to_string(),
            rows.to_string(),
        ];
        self.transport.exec(&argv).await?;
        Ok(())
    }

    /// Find a pane by tmux target string or pane id within a session's panes.
    pub async fn find_pane(&self, session: &str, needle: &str) -> Result<PaneInfo> {
        let panes = self.list_panes(session).await?;
        panes
            .iter()
            .find(|p| p.id.as_str() == needle)
            .or_else(|| {
                panes.iter().find(|p| {
                    format!("{}:{}.{}", p.session, p.window_index, p.pane_index) == needle
                })
            })
            .cloned()
            .ok_or_else(|| EngineError::NotFound(format!("pane {needle} in session {session}")))
    }
}
