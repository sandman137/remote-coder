//! §10.5 privacy test: run the real watch daemon against a fake agent whose
//! pane is full of code-looking text, capture the bytes actually delivered
//! to the (stub) ntfy server, and assert the payload carries exactly the
//! whitelisted fields and none of the pane content. Plus the tier-3 →
//! notify acceptance: an attention match pushes when no regular client is
//! attached.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn tmux_available() -> bool {
    if std::env::var_os("RC_SKIP_TMUX_TESTS").is_some() {
        return false;
    }
    Command::new("tmux")
        .arg("-V")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/agents")
        .join(name)
        .canonicalize()
        .expect("fixture path")
}

struct TmuxServer {
    socket: String,
}

impl TmuxServer {
    fn start(hint: &str) -> Self {
        Self {
            socket: format!("rc-notify-{hint}-{}", std::process::id()),
        }
    }
    fn run(&self, args: &[&str]) {
        assert!(Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .status()
            .unwrap()
            .success());
    }
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .output();
    }
}

/// Accept one HTTP request, return (request head, body), answer 200.
async fn capture_one_post(listener: &TcpListener) -> (String, String) {
    let (mut stream, _) = listener.accept().await.expect("accept");
    let mut raw = Vec::new();
    // Read until the connection closes (sink sends Connection: close).
    let mut buf = [0u8; 4096];
    loop {
        match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                raw.extend_from_slice(&buf[..n]);
                // Full body present? head + content-length check.
                let text = String::from_utf8_lossy(&raw);
                if let Some((head, body)) = text.split_once("\r\n\r\n") {
                    let len = head
                        .lines()
                        .find_map(|l| l.strip_prefix("Content-Length: "))
                        .and_then(|v| v.parse::<usize>().ok())
                        .unwrap_or(0);
                    if body.len() >= len {
                        break;
                    }
                }
            }
            _ => break,
        }
    }
    stream
        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        .await
        .ok();
    let text = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = text.split_once("\r\n\r\n").unwrap_or((text.as_str(), ""));
    (head.to_string(), body.to_string())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tier3_pushes_code_free_payload_when_unattached() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("privacy");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "90",
        "-y",
        "28",
        fixture("fake-yn.sh").to_str().unwrap(),
    ]);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let sink = std::sync::Arc::new(
        notifier::NtfySink::from_url(&format!("http://127.0.0.1:{port}"), "rc-test").unwrap(),
    );

    // The daemon under test (library entry — the same code `notifier watch` runs).
    let daemon = tokio::spawn(notifier::watch::run(
        notifier::watch::WatchConfig {
            socket: Some(server.socket.clone()),
            session: "agents".into(),
            silence_secs: 0, // isolate tier 3 here
            dedupe_secs: 30,
        },
        sink,
    ));

    let (head, body) = tokio::time::timeout(Duration::from_secs(20), capture_one_post(&listener))
        .await
        .expect("no push arrived");
    daemon.abort();

    // Delivered where we said, with the content-free title.
    assert!(head.starts_with("POST /rc-test HTTP/1.1"), "head: {head}");
    assert!(head.contains("Title: fake-yn needs input"), "head: {head}");

    // Exactly the whitelisted fields (§8.5)…
    let value: serde_json::Value = serde_json::from_str(&body).unwrap_or_else(|e| {
        panic!("body is not the payload JSON: {e}\nbody: {body}");
    });
    let obj = value.as_object().unwrap();
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, vec!["agent", "pane", "session", "state"]);
    assert_eq!(obj["session"], "agents");
    assert_eq!(obj["pane"], "%0");
    assert_eq!(obj["state"], "waiting");
    assert_eq!(obj["agent"], "fake-yn");

    // …and none of the pane's actual content (known fake-agent markers).
    for marker in [
        "Proceed",
        "(y/n)",
        "working on task",
        "tokens:",
        "step complete",
    ] {
        assert!(
            !body.contains(marker) && !head.contains(marker),
            "pane content {marker:?} leaked into the push:\n{head}\n{body}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tier2_silence_pushes_after_stall() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("silence");
    // An "agent" that produces output then stalls forever, matching no
    // tier-3 pattern (claude-code adapter detected by command name? no —
    // plain shell). Use a script name matching the fake-yn adapter so an
    // agent is associated, but text that never matches its prompts.
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "80",
        "-y",
        "24",
        "echo compiling module alpha; echo linking; sleep 600",
    ]);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let sink = std::sync::Arc::new(
        notifier::NtfySink::from_url(&format!("http://127.0.0.1:{port}"), "rc-test").unwrap(),
    );

    let daemon = tokio::spawn(notifier::watch::run(
        notifier::watch::WatchConfig {
            socket: Some(server.socket.clone()),
            session: "agents".into(),
            silence_secs: 2,
            dedupe_secs: 30,
        },
        sink,
    ));

    let (_, body) = tokio::time::timeout(Duration::from_secs(20), capture_one_post(&listener))
        .await
        .expect("no silence push arrived");
    daemon.abort();

    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["state"], "waiting");
    assert_eq!(value["session"], "agents");
    assert!(!body.contains("compiling"), "content leaked: {body}");
}
