//! UniFFI surface for the HELM engine (DESIGN.md §7.3/§7.4, §12 Phase 8).
//!
//! The `HelmEngine` object owns a private tokio runtime and drives the real
//! `engine::Engine`; foreign methods are synchronous from UniFFI's view
//! (implemented with `block_on` on the internal runtime — the robust,
//! async-support-independent path from §7.4). Events reach native code two
//! ways: a callback interface `EngineListener` (the preferred push model)
//! and `poll_events()` (the pull fallback). Both are wired so a binding can
//! choose either.

mod types;

pub use types::*;

use std::sync::{Arc, Mutex};

use engine::{ConnConfig, Engine, KeyInput, PaneId, SshParams};
use tokio::runtime::Runtime;

uniffi::setup_scaffolding!();

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum FfiError {
    #[error("engine error: {message}")]
    Engine { message: String },
    #[error("runtime error: {message}")]
    Runtime { message: String },
}

impl From<engine::EngineError> for FfiError {
    fn from(e: engine::EngineError) -> Self {
        FfiError::Engine {
            message: e.to_string(),
        }
    }
}

/// Native side implements this to receive engine events (§7.4).
#[uniffi::export(with_foreign)]
pub trait EngineListener: Send + Sync {
    fn on_event(&self, event: EngineEventFfi);
}

#[derive(uniffi::Object)]
pub struct HelmEngine {
    runtime: Runtime,
    engine: Engine,
    /// Pull-model buffer, drained by `poll_events`.
    buffer: Arc<Mutex<Vec<EngineEventFfi>>>,
    /// Push-model listener, if registered.
    listener: Arc<Mutex<Option<Arc<dyn EngineListener>>>>,
    /// Background event-forwarding task.
    _forwarder: tokio::task::JoinHandle<()>,
}

#[uniffi::export]
impl HelmEngine {
    /// Connect and start forwarding events to the buffer (and any listener).
    #[uniffi::constructor]
    pub fn connect(config: ConnConfigFfi) -> Result<Arc<HelmEngine>, FfiError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .map_err(|e| FfiError::Runtime {
                message: e.to_string(),
            })?;

        let conn = match config {
            ConnConfigFfi::Local { socket } => ConnConfig::Local { socket },
            ConnConfigFfi::Ssh {
                host,
                port,
                user,
                key_path,
                hostkey_fp,
            } => {
                let mut params = SshParams::new(host, port, user, key_path);
                params.hostkey_fp = hostkey_fp;
                ConnConfig::Ssh(params)
            }
        };

        let engine = runtime.block_on(Engine::connect(conn))?;

        let buffer: Arc<Mutex<Vec<EngineEventFfi>>> = Arc::new(Mutex::new(Vec::new()));
        let listener: Arc<Mutex<Option<Arc<dyn EngineListener>>>> = Arc::new(Mutex::new(None));

        let mut events = engine.subscribe();
        let fwd_buffer = Arc::clone(&buffer);
        let fwd_listener = Arc::clone(&listener);
        let forwarder = runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let ffi = EngineEventFfi::from(event);
                        if let Some(l) = fwd_listener.lock().expect("listener lock").clone() {
                            l.on_event(ffi.clone());
                        }
                        let mut buf = fwd_buffer.lock().expect("buffer lock");
                        buf.push(ffi);
                        // Bound the pull buffer so a native side that never
                        // polls can't grow it without limit.
                        if buf.len() > 1024 {
                            let excess = buf.len() - 1024;
                            buf.drain(0..excess);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });

        Ok(Arc::new(HelmEngine {
            runtime,
            engine,
            buffer,
            listener,
            _forwarder: forwarder,
        }))
    }

    pub fn set_listener(&self, listener: Arc<dyn EngineListener>) {
        *self.listener.lock().expect("listener lock") = Some(listener);
    }

    /// Drain buffered events (pull model).
    pub fn poll_events(&self) -> Vec<EngineEventFfi> {
        std::mem::take(&mut *self.buffer.lock().expect("buffer lock"))
    }

    pub fn list_sessions(&self) -> Result<Vec<SessionInfoFfi>, FfiError> {
        let sessions = self.runtime.block_on(self.engine.list_sessions())?;
        Ok(sessions.iter().map(SessionInfoFfi::from).collect())
    }

    pub fn list_panes(&self, session: String) -> Result<Vec<PaneInfoFfi>, FfiError> {
        let panes = self.runtime.block_on(self.engine.list_panes(&session))?;
        Ok(panes.iter().map(PaneInfoFfi::from).collect())
    }

    pub fn snapshot(&self, pane: String, scrollback: u32) -> Result<GridSnapshotFfi, FfiError> {
        let grid = self
            .runtime
            .block_on(self.engine.snapshot(&PaneId(pane), scrollback))?;
        Ok(GridSnapshotFfi::from(&grid))
    }

    /// Send keys using the `<Name>` convention ("y<Enter>", "<C-c>", text).
    pub fn send_keys(&self, pane: String, keys: String) -> Result<(), FfiError> {
        self.runtime
            .block_on(self.engine.send_key_string(&PaneId(pane), &keys))?;
        Ok(())
    }

    /// Send literal text followed by Enter (never re-parsed — safe for text
    /// containing `<`).
    pub fn send_text(&self, pane: String, text: String) -> Result<(), FfiError> {
        let inputs = [
            KeyInput::Text(text),
            KeyInput::Named(vec!["Enter".to_string()]),
        ];
        self.runtime
            .block_on(self.engine.send_keys(&PaneId(pane), &inputs))?;
        Ok(())
    }

    pub fn press_button(&self, pane: String, label: String) -> Result<(), FfiError> {
        self.runtime
            .block_on(self.engine.press_button(&PaneId(pane), &label))?;
        Ok(())
    }

    /// Begin streaming a pane at the given viewport (Grid events follow).
    pub fn attach(&self, pane: String, cols: u16, rows: u16) -> Result<(), FfiError> {
        self.runtime
            .block_on(self.engine.attach(&PaneId(pane), (cols, rows)))?;
        Ok(())
    }

    pub fn detach(&self, pane: String) -> Result<(), FfiError> {
        self.runtime.block_on(self.engine.detach(&PaneId(pane)))?;
        Ok(())
    }

    pub fn resize(&self, pane: String, cols: u16, rows: u16) -> Result<(), FfiError> {
        self.runtime
            .block_on(self.engine.resize(&PaneId(pane), cols, rows))?;
        Ok(())
    }

    pub fn launch_agent(
        &self,
        session: String,
        adapter_id: String,
        cwd: Option<String>,
    ) -> Result<String, FfiError> {
        let pane = self
            .runtime
            .block_on(self.engine.launch_agent(&session, &adapter_id, cwd))?;
        Ok(pane.0)
    }
}

/// Engine version (smoke check the .so loaded and links).
#[uniffi::export]
pub fn engine_version() -> String {
    engine::version().to_string()
}
