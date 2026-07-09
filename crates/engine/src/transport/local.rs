//! `LocalTransport` — drives tmux on the same host via subprocess. The default
//! transport for tests and the TUI harness; exercises 100% of the tmux
//! protocol, grid, adapter, and attention code paths with no network or keys.

use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use super::{ControlChannel, Transport};
use crate::error::TransportError;

/// How long a one-shot tmux command may run before we consider it wedged.
/// Generous: `capture-pane` on huge scrollback is the slow case.
const EXEC_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Default)]
pub struct LocalTransport {
    /// tmux binary; default `tmux` from PATH.
    tmux_bin: String,
    /// `tmux -L <socket>` server socket name; `None` = default server.
    socket: Option<String>,
}

impl LocalTransport {
    pub fn new(socket: Option<String>) -> Self {
        Self {
            tmux_bin: "tmux".to_string(),
            socket,
        }
    }

    fn base_argv(&self) -> Vec<String> {
        let mut argv = Vec::new();
        if let Some(sock) = &self.socket {
            argv.push("-L".to_string());
            argv.push(sock.clone());
        }
        argv
    }
}

#[async_trait::async_trait]
impl Transport for LocalTransport {
    async fn exec(&self, argv: &[String]) -> Result<Vec<u8>, TransportError> {
        let mut cmd = Command::new(&self.tmux_bin);
        cmd.args(self.base_argv())
            .args(argv)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        tracing::trace!(?argv, "local exec");
        let run = async {
            let out = cmd.spawn()?.wait_with_output().await?;
            if out.status.success() {
                Ok(out.stdout)
            } else {
                Err(TransportError::Tmux {
                    status: out.status.code().unwrap_or(-1),
                    stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
                })
            }
        };
        tokio::time::timeout(EXEC_TIMEOUT, run)
            .await
            .map_err(|_| TransportError::Timeout("tmux exec"))?
    }

    async fn open_control(
        &self,
        _session: &str,
        _size: (u16, u16),
    ) -> Result<Box<dyn ControlChannel>, TransportError> {
        // Lands in Phase 3 (control-mode streaming).
        Err(TransportError::Unsupported(
            "LocalTransport::open_control arrives in Phase 3",
        ))
    }
}
