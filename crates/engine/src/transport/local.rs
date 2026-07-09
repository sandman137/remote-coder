//! `LocalTransport` — drives tmux on the same host via subprocess. The default
//! transport for tests and the TUI harness; exercises 100% of the tmux
//! protocol, grid, adapter, and attention code paths with no network or keys.

use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;
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
        session: &str,
        _size: (u16, u16),
    ) -> Result<Box<dyn ControlChannel>, TransportError> {
        // Client size is set by the streamer via `refresh-client -C` right
        // after attach (control clients have no tty to size).
        let mut cmd = Command::new(&self.tmux_bin);
        cmd.args(self.base_argv())
            .arg("-C")
            .arg("attach-session")
            .arg("-t")
            .arg(format!("={session}"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true);

        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or(TransportError::Closed)?;
        let stdout = child.stdout.take().ok_or(TransportError::Closed)?;
        Ok(Box::new(LocalControlChannel {
            _child: child,
            stdin,
            stdout,
            acc: Vec::with_capacity(8192),
            eof: false,
        }))
    }
}

struct LocalControlChannel {
    /// Held for kill_on_drop — dropping the channel ends the tmux client.
    _child: tokio::process::Child,
    stdin: tokio::process::ChildStdin,
    stdout: tokio::process::ChildStdout,
    /// Accumulator that survives cancelled `read_line` futures (see the
    /// cancellation-safety contract on `ControlChannel::read_line`).
    acc: Vec<u8>,
    eof: bool,
}

impl LocalControlChannel {
    /// Pop one complete line (without newline) from the accumulator.
    fn take_line(&mut self) -> Option<Vec<u8>> {
        let pos = self.acc.iter().position(|&b| b == b'\n')?;
        let mut line: Vec<u8> = self.acc.drain(..=pos).collect();
        line.pop(); // the newline
        Some(line)
    }
}

#[async_trait::async_trait]
impl ControlChannel for LocalControlChannel {
    async fn write_line(&mut self, line: &str) -> Result<(), TransportError> {
        self.stdin.write_all(line.as_bytes()).await?;
        self.stdin.write_all(b"\n").await?;
        self.stdin.flush().await?;
        Ok(())
    }

    async fn read_line(&mut self) -> Result<Option<Vec<u8>>, TransportError> {
        use tokio::io::AsyncReadExt;
        loop {
            if let Some(line) = self.take_line() {
                return Ok(Some(line));
            }
            if self.eof {
                // Final unterminated fragment, then EOF forever.
                if self.acc.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(std::mem::take(&mut self.acc)));
            }
            // Single read syscall per await point: cancellation-safe.
            let n = self.stdout.read_buf(&mut self.acc).await?;
            if n == 0 {
                self.eof = true;
            }
        }
    }
}
