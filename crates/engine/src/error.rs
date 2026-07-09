//! Error types shared across the engine.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("failed to spawn process: {0}")]
    Spawn(#[from] std::io::Error),

    #[error("tmux exited with status {status}: {stderr}")]
    Tmux { status: i32, stderr: String },

    #[error("operation not supported by this transport: {0}")]
    Unsupported(&'static str),

    #[error("protocol error: {0}")]
    Protocol(String),

    #[error("channel closed")]
    Closed,

    #[error("connection failed: {0}")]
    Connect(String),

    #[error("authentication failed: {0}")]
    Auth(String),

    #[error("host key mismatch: pinned {pinned}, presented {presented}")]
    HostKeyMismatch { pinned: String, presented: String },

    #[error("operation timed out: {0}")]
    Timeout(&'static str),
}

#[derive(Debug, Error)]
pub enum EngineError {
    #[error(transparent)]
    Transport(#[from] TransportError),

    #[error("failed to parse tmux output: {0}")]
    Parse(String),

    #[error("not found: {0}")]
    NotFound(String),

    #[error("unknown adapter: {0}")]
    UnknownAdapter(String),

    #[error("unknown button {button:?} for adapter {adapter}")]
    UnknownButton { adapter: String, button: String },

    #[error("invalid input: {0}")]
    InvalidInput(String),

    #[error("not attached to pane {0}")]
    NotAttached(String),
}

pub type Result<T, E = EngineError> = std::result::Result<T, E>;
