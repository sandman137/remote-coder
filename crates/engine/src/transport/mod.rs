//! The `Transport` abstraction (DESIGN.md §3): everything the engine does is
//! either a one-shot tmux command (`exec`) or the long-lived control-mode
//! channel (`open_control`). `LocalTransport` drives tmux on this host;
//! `SshTransport` (Phase 5) does the same over russh. Because the engine only
//! ever sees this trait, the TUI harness, tests, and mobile apps all exercise
//! identical code.

mod local;
mod ssh;

pub use local::LocalTransport;
pub(crate) use ssh::shell_quote;
pub use ssh::{SshParams, SshTransport};

use crate::error::TransportError;

/// One-shot tmux invocations and the streaming control-mode channel.
///
/// `argv` is the tmux *subcommand* argv (e.g. `["list-panes", "-a", "-F", …]`)
/// — no `tmux` prefix, no socket/server flags; each transport decides how the
/// tmux binary is reached. Argv is always passed as an array, never a shell
/// string (DESIGN.md §13 send-keys safety).
#[async_trait::async_trait]
pub trait Transport: Send + Sync {
    /// Run a single tmux invocation, returning its stdout on exit status 0.
    async fn exec(&self, argv: &[String]) -> Result<Vec<u8>, TransportError>;

    /// Open the streaming control-mode channel:
    /// `tmux -C attach-session -t <session>` sized to `(cols, rows)`.
    async fn open_control(
        &self,
        session: &str,
        size: (u16, u16),
    ) -> Result<Box<dyn ControlChannel>, TransportError>;
}

/// Duplex line channel for tmux control mode: we write tmux commands as
/// lines, we read `%`-notification lines (octal escapes intact — decoding
/// is the protocol layer's job, not the transport's).
#[async_trait::async_trait]
pub trait ControlChannel: Send {
    async fn write_line(&mut self, line: &str) -> Result<(), TransportError>;

    /// Next raw control-mode line without its trailing newline.
    /// `Ok(None)` = orderly EOF.
    ///
    /// CONTRACT: implementations MUST be cancellation-safe — the streamer
    /// polls this inside `tokio::select!`, so the future is routinely dropped
    /// mid-read. Partial bytes must survive in the implementation's buffer,
    /// never be discarded. (Use a persistent accumulator + `read_buf`, not
    /// `read_until` with a cleared buffer.)
    async fn read_line(&mut self) -> Result<Option<Vec<u8>>, TransportError>;
}
