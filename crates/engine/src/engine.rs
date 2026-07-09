//! The `Engine` facade (DESIGN.md §7.1) — the one public entrypoint UIs use.

use std::collections::HashMap;
use std::sync::Arc;

use crate::adapter::{AgentAdapter, Registry};
use crate::error::{EngineError, Result, TransportError};
use crate::event::{EventBus, EventStream};
use crate::grid::{sgr, GridSnapshot};
use crate::stream::{self, StreamCmd, StreamHandle};
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
    /// One control-mode streamer per attached session (Phase 3).
    streamers: tokio::sync::Mutex<HashMap<String, StreamHandle>>,
    registry: Arc<Registry>,
}

impl Engine {
    pub async fn connect(cfg: ConnConfig) -> Result<Engine> {
        let registry = Registry::load_builtins_and_overrides().unwrap_or_else(|e| {
            tracing::warn!(error = %e, "adapter overrides failed; using builtins");
            Registry::load_builtins().expect("builtin adapters must parse")
        });
        Self::connect_with_registry(cfg, registry).await
    }

    /// Dependency-injected variant (tests, custom adapter dirs).
    pub async fn connect_with_registry(cfg: ConnConfig, registry: Registry) -> Result<Engine> {
        let transport: Arc<dyn Transport> = match cfg {
            ConnConfig::Local { socket } => Arc::new(LocalTransport::new(socket)),
            ConnConfig::Ssh { .. } => {
                return Err(TransportError::Unsupported("SshTransport arrives in Phase 5").into())
            }
        };
        Ok(Engine {
            transport,
            events: EventBus::new(),
            streamers: tokio::sync::Mutex::new(HashMap::new()),
            registry: Arc::new(registry),
        })
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
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

    /// Begin streaming a pane: attaches a control-mode client to the pane's
    /// session (shared per session), sets the client size so tmux reflows for
    /// this viewport (§4.3), and marks the pane watched — `EngineEvent::Grid`
    /// with dirty rows flows from here on.
    pub async fn attach(&self, pane: &PaneId, size: (u16, u16)) -> Result<()> {
        let session = self.resolve_session(pane).await?;
        let mut streamers = self.streamers.lock().await;
        // Replace a dead streamer (e.g. reconnect exhausted).
        if streamers.get(&session).is_some_and(|h| !h.is_alive()) {
            streamers.remove(&session);
        }
        if !streamers.contains_key(&session) {
            let handle = stream::spawn(
                Arc::clone(&self.transport),
                session.clone(),
                size,
                self.events.clone(),
                Arc::clone(&self.registry),
            )
            .await?;
            streamers.insert(session.clone(), handle);
        }
        let handle = streamers.get(&session).expect("just inserted");
        let _ = handle.cmd_tx.send(StreamCmd::SetSize {
            cols: size.0,
            rows: size.1,
        });
        let _ = handle.cmd_tx.send(StreamCmd::Watch(pane.clone()));
        Ok(())
    }

    /// Stop emitting Grid events for a pane. The session streamer stays warm
    /// for quick re-attach; it dies with the Engine.
    pub async fn detach(&self, pane: &PaneId) -> Result<()> {
        if let Ok(session) = self.resolve_session(pane).await {
            if let Some(handle) = self.streamers.lock().await.get(&session) {
                let _ = handle.cmd_tx.send(StreamCmd::Unwatch(pane.clone()));
            }
        }
        Ok(())
    }

    /// Resize for reflow. With a streamer attached this sets the *client*
    /// size (`refresh-client -C`) — tmux reflows the window to the latest
    /// client (§4.3). Without one it falls back to `resize-window`.
    pub async fn resize(&self, pane: &PaneId, cols: u16, rows: u16) -> Result<()> {
        if let Ok(session) = self.resolve_session(pane).await {
            if let Some(handle) = self.streamers.lock().await.get(&session) {
                if handle.is_alive() {
                    let _ = handle.cmd_tx.send(StreamCmd::SetSize { cols, rows });
                    return Ok(());
                }
            }
        }
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

    /// The adapter driving a pane, if detectable: by foreground command
    /// first, then by prompt patterns in the visible text (§6.2).
    pub async fn adapter_for_pane(&self, pane: &PaneId) -> Result<Option<AgentAdapter>> {
        let out = self.transport.exec(&cmd::list_panes(None)).await?;
        let panes = parse_panes(&out)?;
        let Some(info) = panes.iter().find(|p| {
            p.id == *pane || format!("{}:{}.{}", p.session, p.window_index, p.pane_index) == pane.0
        }) else {
            return Err(EngineError::NotFound(format!("pane {pane}")));
        };
        if let Some(a) = self.registry.detect(&info.current_command, "") {
            return Ok(Some(a.clone()));
        }
        let text = self.snapshot(&info.id, 0).await?.to_text();
        Ok(self.registry.detect(&info.current_command, &text).cloned())
    }

    /// Press a named adapter button on a pane (§7.1): resolves the pane's
    /// adapter, looks the label up, sends its keys.
    pub async fn press_button(&self, pane: &PaneId, button_label: &str) -> Result<()> {
        let adapter = self
            .adapter_for_pane(pane)
            .await?
            .ok_or_else(|| EngineError::UnknownAdapter(format!("no adapter for pane {pane}")))?;
        let button = adapter
            .buttons
            .iter()
            .find(|b| b.label.eq_ignore_ascii_case(button_label))
            .ok_or_else(|| EngineError::UnknownButton {
                adapter: adapter.id.clone(),
                button: button_label.to_string(),
            })?;
        self.send_key_string(pane, &button.keys).await
    }

    /// Launch an agent in a fresh window of `session` (§7.1). `cwd`
    /// overrides the adapter's policy; a "picker" adapter without a cwd
    /// falls back to the remote user's home (tmux default).
    pub async fn launch_agent(
        &self,
        session: &str,
        adapter_id: &str,
        cwd: Option<String>,
    ) -> Result<PaneId> {
        let adapter = self
            .registry
            .get(adapter_id)
            .ok_or_else(|| EngineError::UnknownAdapter(adapter_id.to_string()))?;
        let cwd = cwd.or(match &adapter.launch.cwd {
            crate::adapter::CwdPolicy::Fixed(path) => Some(path.clone()),
            crate::adapter::CwdPolicy::Picker => None,
        });
        // Command line for the new pane. Args are shell-quoted defensively;
        // the launch cmd itself comes from trusted adapter config.
        let mut shell_cmd = adapter.launch.cmd.clone();
        for arg in &adapter.launch.args {
            shell_cmd.push(' ');
            shell_cmd.push_str(&shell_quote(arg));
        }
        let argv = cmd::new_window(session, &adapter.id, cwd.as_deref(), &shell_cmd);
        let out = self.transport.exec(&argv).await?;
        let id = String::from_utf8_lossy(&out).trim().to_string();
        if id.is_empty() {
            return Err(EngineError::Parse("new-window returned no pane id".into()));
        }
        Ok(PaneId(id))
    }

    /// Which session a pane belongs to. `sess:win.pane` targets parse
    /// directly; `%id` panes are looked up across all sessions.
    async fn resolve_session(&self, pane: &PaneId) -> Result<String> {
        if let Some((session, _)) = pane.0.split_once(':') {
            return Ok(session.trim_start_matches('=').to_string());
        }
        let out = self.transport.exec(&cmd::list_panes(None)).await?;
        parse_panes(&out)?
            .into_iter()
            .find(|p| p.id == *pane)
            .map(|p| p.session)
            .ok_or_else(|| EngineError::NotFound(format!("pane {pane}")))
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

/// Single-quote a string for the POSIX shell that tmux hands new-window
/// commands to.
fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}
