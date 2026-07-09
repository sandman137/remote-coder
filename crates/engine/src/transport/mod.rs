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

    /// Store `data` as an attachment named `name` on the host side and return
    /// the absolute path (for insertion into an agent prompt). Over SSH this
    /// is the broker's `upload` verb; locally it writes `~/.rcoder/uploads`.
    async fn upload(&self, name: &str, data: &[u8]) -> Result<String, TransportError>;
}

/// Shared local-side implementation of the uploads dir contract (also what
/// the broker enforces remotely): `~/.rcoder/uploads`, epoch-prefixed
/// sanitized name, never overwrites.
pub(crate) fn write_upload_local(name: &str, data: &[u8]) -> Result<String, TransportError> {
    // Keep in lockstep with broker::sanitize_upload_name — paths land inside
    // agent prompts, so spaces/hostile chars must never survive.
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    let cleaned: String = base
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(80)
        .collect();
    let cleaned = cleaned.trim_start_matches('.').to_string();
    if cleaned.is_empty() {
        return Err(TransportError::Protocol(format!(
            "unusable attachment name {name:?}"
        )));
    }
    let home = std::env::var("HOME")
        .map_err(|_| TransportError::Protocol("HOME not set".into()))?;
    let dir = std::path::Path::new(&home).join(".rcoder").join("uploads");
    std::fs::create_dir_all(&dir).map_err(|e| TransportError::Protocol(e.to_string()))?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{stamp}-{cleaned}"));
    std::fs::write(&path, data).map_err(|e| TransportError::Protocol(e.to_string()))?;
    Ok(path.display().to_string())
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
