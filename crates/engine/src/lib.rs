//! HELM engine — the platform-agnostic core.
//!
//! Everything security-critical and platform-independent lives here:
//! transport (local/SSH), tmux protocol, grid model, agent adapters,
//! attention detection. Native UIs (ratatui harness, Android, iOS)
//! consume this crate — directly or through `engine-ffi`.
//!
//! Log discipline (DESIGN.md §13): pane content appears only at `trace`
//! level; `info` and below must never carry code or prompt text.

pub mod adapter;
pub mod attention;
mod engine;
pub mod error;
pub mod event;
pub mod grid;
pub mod security;
mod stream;
pub mod tmux;
pub mod transport;

pub use adapter::{AgentAdapter, Registry};
pub use engine::{ConnConfig, Engine};
pub use error::{EngineError, Result, TransportError};
pub use event::{Button, EngineEvent, EventStream, PromptKind};
pub use grid::{Cell, CellAttrs, Color, GridSnapshot};
pub use security::{pairing::PairPayload, FileKeyStore, KeyStore};
pub use tmux::keys::{parse_key_string, KeyInput};
pub use tmux::{PaneId, PaneInfo, SessionInfo};
pub use transport::SshParams;

pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
