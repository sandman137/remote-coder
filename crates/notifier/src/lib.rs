//! Notifier (DESIGN.md §9): host-side push with privacy by construction.
//!
//! THE invariant (§8.5): a push payload carries only
//! `{session, pane, state, agent}` — never pane text, code, prompts, or
//! file paths, because payloads transit FCM/APNs/ntfy infrastructure. The
//! `Payload` type has exactly those fields and no free-form constructor
//! input can add more; the privacy test (§10.5) checks the delivered bytes.

pub mod watch;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Waiting,
    Done,
    Error,
}

impl std::str::FromStr for AgentState {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "waiting" => Ok(AgentState::Waiting),
            "done" => Ok(AgentState::Done),
            "error" => Ok(AgentState::Error),
            other => Err(format!("unknown state {other:?} (waiting|done|error)")),
        }
    }
}

/// The complete push payload. Field set is the §8.5 whitelist — do not add
/// fields that could carry content.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub session: String,
    pub pane: String,
    pub state: AgentState,
    pub agent: String,
}

impl Payload {
    pub fn new(session: &str, pane: &str, state: AgentState, agent: &str) -> Self {
        Payload {
            session: session.to_string(),
            pane: pane.to_string(),
            state,
            agent: agent.to_string(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("payload serializes")
    }

    /// Human title for notification UIs — still content-free.
    pub fn title(&self) -> String {
        let verb = match self.state {
            AgentState::Waiting => "needs input",
            AgentState::Done => "finished",
            AgentState::Error => "errored",
        };
        format!("{} {}", self.agent, verb)
    }
}

#[async_trait::async_trait]
pub trait Sink: Send + Sync {
    async fn send(&self, payload: &Payload) -> Result<(), SinkError>;
    fn name(&self) -> &'static str;
}

#[derive(Debug, thiserror::Error)]
pub enum SinkError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Other(String),
}

/// Self-hosted ntfy over plain HTTP (dev + tailnet-internal). Deliberately
/// dependency-free: one POST, one status line.
pub struct NtfySink {
    host: String,
    port: u16,
    topic: String,
}

impl NtfySink {
    /// `url` like `http://127.0.0.1:2586`; https is refused (the dev/tailnet
    /// sink is plaintext-internal by design — public ntfy would need TLS).
    pub fn from_url(url: &str, topic: &str) -> Result<Self, SinkError> {
        let rest = url
            .strip_prefix("http://")
            .ok_or_else(|| SinkError::Other(format!("ntfy url must be http:// (got {url})")))?;
        let rest = rest.trim_end_matches('/');
        let (host, port) = match rest.split_once(':') {
            Some((h, p)) => (
                h.to_string(),
                p.parse::<u16>()
                    .map_err(|_| SinkError::Other(format!("bad port in {url}")))?,
            ),
            None => (rest.to_string(), 80),
        };
        Ok(NtfySink {
            host,
            port,
            topic: topic.to_string(),
        })
    }
}

#[async_trait::async_trait]
impl Sink for NtfySink {
    async fn send(&self, payload: &Payload) -> Result<(), SinkError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let body = payload.to_json();
        let request = format!(
            "POST /{topic} HTTP/1.1\r\nHost: {host}\r\nTitle: {title}\r\n\
             Content-Type: application/json\r\nContent-Length: {len}\r\n\
             Connection: close\r\n\r\n{body}",
            topic = self.topic,
            host = self.host,
            title = payload.title(),
            len = body.len(),
        );
        let mut stream = tokio::net::TcpStream::connect((self.host.as_str(), self.port)).await?;
        stream.write_all(request.as_bytes()).await?;

        let mut response = Vec::new();
        stream.read_to_end(&mut response).await?;
        let status = String::from_utf8_lossy(&response);
        let ok = status.starts_with("HTTP/1.1 2") || status.starts_with("HTTP/1.0 2");
        if !ok {
            let line = status.lines().next().unwrap_or("<empty>");
            return Err(SinkError::Other(format!("ntfy rejected: {line}")));
        }
        Ok(())
    }

    fn name(&self) -> &'static str {
        "ntfy"
    }
}

/// FCM stub (Phase 9 wires the real sender): spools payload JSON lines so
/// the Android bring-up can drain and verify them.
pub struct FcmStubSink {
    pub spool: std::path::PathBuf,
}

#[async_trait::async_trait]
impl Sink for FcmStubSink {
    async fn send(&self, payload: &Payload) -> Result<(), SinkError> {
        use tokio::io::AsyncWriteExt;
        if let Some(parent) = self.spool.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.spool)
            .await?;
        file.write_all(format!("{}\n", payload.to_json()).as_bytes())
            .await?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "fcm-stub"
    }
}

/// Prints payloads to stdout (debugging).
pub struct StdoutSink;

#[async_trait::async_trait]
impl Sink for StdoutSink {
    async fn send(&self, payload: &Payload) -> Result<(), SinkError> {
        println!("{}", payload.to_json());
        Ok(())
    }
    fn name(&self) -> &'static str {
        "stdout"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_has_exactly_the_whitelisted_fields() {
        let p = Payload::new("agents", "%3", AgentState::Waiting, "claude-code");
        let json = p.to_json();
        let value: serde_json::Value = serde_json::from_str(&json).unwrap();
        let obj = value.as_object().unwrap();
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["agent", "pane", "session", "state"]);
        assert_eq!(obj["state"], "waiting");
    }

    #[test]
    fn payload_rejects_extra_fields_on_ingest() {
        let smuggled =
            r#"{"session":"s","pane":"%1","state":"done","agent":"a","code":"fn x(){}"}"#;
        assert!(serde_json::from_str::<Payload>(smuggled).is_err());
    }

    #[test]
    fn title_is_content_free() {
        let p = Payload::new("agents", "%3", AgentState::Waiting, "codex");
        assert_eq!(p.title(), "codex needs input");
    }

    #[test]
    fn ntfy_url_parsing() {
        assert!(NtfySink::from_url("http://127.0.0.1:2586", "helm").is_ok());
        assert!(NtfySink::from_url("http://ntfy.internal", "helm").is_ok());
        assert!(NtfySink::from_url("https://ntfy.sh", "helm").is_err());
        assert!(NtfySink::from_url("ftp://x", "helm").is_err());
    }
}
