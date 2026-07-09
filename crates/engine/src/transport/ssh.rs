//! `SshTransport` — the real remote path (DESIGN.md §3.2): russh with
//! key-only auth and host-key pinning enforced in `check_server_key`.
//! `exec` runs one tmux invocation per SSH exec channel; `open_control`
//! runs `tmux -C attach-session` on a long-lived channel.
//!
//! Proven against loopback sshd (tests/loopback_ssh.rs) — same code path a
//! phone uses over the tailnet.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{load_secret_key, HashAlg, PrivateKeyWithHashAlg, PublicKey};
use russh::{ChannelMsg, Disconnect};
use tokio::io::{AsyncWrite, AsyncWriteExt};

use super::{ControlChannel, Transport};
use crate::error::TransportError;

const EXEC_TIMEOUT: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone)]
pub struct SshParams {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// Path to the ed25519 identity. (The Phase-6 keystore signs in
    /// hardware on mobile; file keys serve the desktop/dev path.)
    pub key_path: String,
    /// Pinned host key fingerprint ("SHA256:…"). `None` = trust-on-first-use:
    /// the observed fingerprint is recorded and surfaced for pinning.
    pub hostkey_fp: Option<String>,
    /// How the remote tmux binary is invoked; the argv from `exec()` is
    /// appended. Defaults to `["tmux"]`. Tests point this at a private
    /// server socket; Phase 6 pairs it with the broker's forced command.
    pub tmux_prefix: Vec<String>,
}

impl SshParams {
    pub fn new(
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
        key_path: impl Into<String>,
    ) -> Self {
        SshParams {
            host: host.into(),
            port,
            user: user.into(),
            key_path: key_path.into(),
            hostkey_fp: None,
            tmux_prefix: vec!["tmux".into()],
        }
    }
}

/// Enforces pinning: with a pinned fingerprint, anything else is rejected
/// (MITM protection even on a hostile network); without one, the observed
/// fingerprint is recorded for the caller to pin (TOFU, §8.1).
struct PinHandler {
    pinned: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl client::Handler for PinHandler {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &PublicKey) -> Result<bool, Self::Error> {
        let fp = key.fingerprint(HashAlg::Sha256).to_string();
        *self.seen.lock().expect("fingerprint mutex") = Some(fp.clone());
        match &self.pinned {
            Some(pinned) => Ok(&fp == pinned),
            None => Ok(true),
        }
    }
}

pub struct SshTransport {
    handle: Handle<PinHandler>,
    tmux_prefix: Vec<String>,
    /// Fingerprint presented by the server this connection.
    server_fp: Option<String>,
}

impl SshTransport {
    pub async fn connect(params: &SshParams) -> Result<Self, TransportError> {
        let config = Arc::new(client::Config {
            keepalive_interval: Some(Duration::from_secs(20)),
            ..Default::default()
        });
        let seen = Arc::new(Mutex::new(None));
        let handler = PinHandler {
            pinned: params.hostkey_fp.clone(),
            seen: Arc::clone(&seen),
        };

        let connect = client::connect(config, (params.host.as_str(), params.port), handler);
        let mut handle = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| TransportError::Timeout("ssh connect"))?
            .map_err(|e| {
                let presented = seen.lock().expect("fingerprint mutex").clone();
                match (&e, &params.hostkey_fp, presented) {
                    (russh::Error::UnknownKey, Some(pinned), presented) => {
                        TransportError::HostKeyMismatch {
                            pinned: pinned.clone(),
                            presented: presented.unwrap_or_else(|| "<none>".into()),
                        }
                    }
                    _ => TransportError::Connect(e.to_string()),
                }
            })?;

        let key = load_secret_key(&params.key_path, None)
            .map_err(|e| TransportError::Auth(format!("load key {}: {e}", params.key_path)))?;
        let auth = handle
            .authenticate_publickey(
                params.user.clone(),
                PrivateKeyWithHashAlg::new(Arc::new(key), None),
            )
            .await
            .map_err(|e| TransportError::Auth(e.to_string()))?;
        if !auth.success() {
            return Err(TransportError::Auth(format!(
                "server rejected public key for user {}",
                params.user
            )));
        }

        let server_fp = seen.lock().expect("fingerprint mutex").clone();
        tracing::info!(
            host = %params.host,
            fingerprint = %server_fp.as_deref().unwrap_or("<unknown>"),
            pinned = params.hostkey_fp.is_some(),
            "ssh connected"
        );
        Ok(SshTransport {
            handle,
            tmux_prefix: params.tmux_prefix.clone(),
            server_fp,
        })
    }

    /// The fingerprint the server presented (pin this after TOFU).
    pub fn server_fingerprint(&self) -> Option<&str> {
        self.server_fp.as_deref()
    }

    pub async fn disconnect(&self) {
        let _ = self
            .handle
            .disconnect(Disconnect::ByApplication, "bye", "en")
            .await;
    }

    /// Build the remote command line: prefix + argv, shell-quoted (the SSH
    /// exec request goes through the remote user's shell).
    fn command_line(&self, argv: &[String]) -> String {
        self.tmux_prefix
            .iter()
            .chain(argv.iter())
            .map(|a| shell_quote(a))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Single-quote for POSIX shells.
pub(crate) fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:%@+,".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[async_trait::async_trait]
impl Transport for SshTransport {
    async fn exec(&self, argv: &[String]) -> Result<Vec<u8>, TransportError> {
        let run = async {
            let mut channel = self
                .handle
                .channel_open_session()
                .await
                .map_err(|e| TransportError::Connect(e.to_string()))?;
            channel
                .exec(true, self.command_line(argv))
                .await
                .map_err(|e| TransportError::Connect(e.to_string()))?;

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut status: Option<u32> = None;
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, ext: 1 } => stderr.extend_from_slice(&data),
                    ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                    _ => {}
                }
            }
            match status {
                Some(0) => Ok(stdout),
                Some(code) => Err(TransportError::Tmux {
                    status: code as i32,
                    stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
                }),
                None => Err(TransportError::Protocol(
                    "ssh exec channel closed without exit status".into(),
                )),
            }
        };
        tokio::time::timeout(EXEC_TIMEOUT, run)
            .await
            .map_err(|_| TransportError::Timeout("ssh exec"))?
    }

    async fn upload(&self, name: &str, data: &[u8]) -> Result<String, TransportError> {
        // The broker's `upload <name> <size>` verb: bytes on stdin, absolute
        // stored path on stdout. Bypasses the tmux prefix on purpose.
        let cmd = format!("upload {} {}", shell_quote(name), data.len());
        let run = async {
            let mut channel = self
                .handle
                .channel_open_session()
                .await
                .map_err(|e| TransportError::Connect(e.to_string()))?;
            channel
                .exec(true, cmd)
                .await
                .map_err(|e| TransportError::Connect(e.to_string()))?;
            channel
                .data(data)
                .await
                .map_err(|e| TransportError::Connect(e.to_string()))?;
            channel
                .eof()
                .await
                .map_err(|e| TransportError::Connect(e.to_string()))?;

            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            let mut status: Option<u32> = None;
            while let Some(msg) = channel.wait().await {
                match msg {
                    ChannelMsg::Data { data } => stdout.extend_from_slice(&data),
                    ChannelMsg::ExtendedData { data, ext: 1 } => stderr.extend_from_slice(&data),
                    ChannelMsg::ExitStatus { exit_status } => status = Some(exit_status),
                    _ => {}
                }
            }
            match status {
                Some(0) => Ok(String::from_utf8_lossy(&stdout).trim().to_string()),
                Some(code) => Err(TransportError::Tmux {
                    status: code as i32,
                    stderr: String::from_utf8_lossy(&stderr).trim().to_string(),
                }),
                None => Err(TransportError::Protocol(
                    "ssh upload channel closed without exit status".into(),
                )),
            }
        };
        // Uploads can be MBs over mobile links — allow well beyond EXEC_TIMEOUT.
        tokio::time::timeout(std::time::Duration::from_secs(120), run)
            .await
            .map_err(|_| TransportError::Timeout("ssh upload"))?
    }

    async fn open_control(
        &self,
        session: &str,
        _size: (u16, u16),
    ) -> Result<Box<dyn ControlChannel>, TransportError> {
        let channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| TransportError::Connect(e.to_string()))?;
        let argv: Vec<String> = vec![
            "-C".into(),
            "attach-session".into(),
            "-t".into(),
            format!("={session}"),
        ];
        channel
            .exec(true, self.command_line(&argv))
            .await
            .map_err(|e| TransportError::Connect(e.to_string()))?;

        let writer = Box::pin(channel.make_writer());
        Ok(Box::new(SshControlChannel {
            channel,
            writer,
            acc: Vec::with_capacity(8192),
            eof: false,
        }))
    }
}

struct SshControlChannel {
    channel: russh::Channel<client::Msg>,
    writer: Pin<Box<dyn AsyncWrite + Send>>,
    /// Cancellation-safe accumulator (see `ControlChannel::read_line`).
    acc: Vec<u8>,
    eof: bool,
}

impl SshControlChannel {
    fn take_line(&mut self) -> Option<Vec<u8>> {
        let pos = self.acc.iter().position(|&b| b == b'\n')?;
        let mut line: Vec<u8> = self.acc.drain(..=pos).collect();
        line.pop();
        Some(line)
    }
}

#[async_trait::async_trait]
impl ControlChannel for SshControlChannel {
    async fn write_line(&mut self, line: &str) -> Result<(), TransportError> {
        self.writer.write_all(line.as_bytes()).await?;
        self.writer.write_all(b"\n").await?;
        self.writer.flush().await?;
        Ok(())
    }

    async fn read_line(&mut self) -> Result<Option<Vec<u8>>, TransportError> {
        loop {
            if let Some(line) = self.take_line() {
                return Ok(Some(line));
            }
            if self.eof {
                if self.acc.is_empty() {
                    return Ok(None);
                }
                return Ok(Some(std::mem::take(&mut self.acc)));
            }
            // channel.wait() is a bounded-mpsc recv: cancellation-safe.
            match self.channel.wait().await {
                Some(ChannelMsg::Data { data }) => self.acc.extend_from_slice(&data),
                Some(ChannelMsg::Eof) | Some(ChannelMsg::Close) | None => self.eof = true,
                Some(_) => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_passes_safe_and_wraps_unsafe() {
        assert_eq!(shell_quote("list-sessions"), "list-sessions");
        assert_eq!(shell_quote("-F"), "-F");
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), r#"'it'\''s'"#);
        assert_eq!(shell_quote(""), "''");
        // Format strings with braces/hashes must be quoted.
        assert_eq!(shell_quote("#{pane_id}"), "'#{pane_id}'");
    }
}
