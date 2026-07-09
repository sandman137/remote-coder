//! Tier-1 hook smoke test: run the shipped claude-code hook script inside a
//! real tmux pane (so $TMUX_PANE + display-message work), pointing at the
//! built notifier binary and a stub ntfy — the §9 hook path end to end.

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hook_script_delivers_waiting_payload() {
    if !tmux_available() {
        return;
    }
    let socket = format!("rc-hook-{}", std::process::id());
    let hook = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("hooks/claude-code-hook.sh")
        .canonicalize()
        .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    // Session named like production; the hook runs as the pane command,
    // exactly as Claude Code would invoke it from inside the pane.
    let shell_cmd = format!(
        "NOTIFIER_BIN={bin} NTFY_URL=http://127.0.0.1:{port} NTFY_TOPIC=hooked {hook}; sleep 30",
        bin = env!("CARGO_BIN_EXE_notifier"),
        hook = hook.display(),
    );
    assert!(Command::new("tmux")
        .args([
            "-L",
            &socket,
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "agents"
        ])
        .arg(shell_cmd)
        .status()
        .unwrap()
        .success());

    // Capture the POST.
    let accept = async {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        let mut buf = [0u8; 2048];
        loop {
            match tokio::time::timeout(Duration::from_secs(5), stream.read(&mut buf)).await {
                Ok(Ok(0)) => break,
                Ok(Ok(n)) => {
                    raw.extend_from_slice(&buf[..n]);
                    if String::from_utf8_lossy(&raw).contains("\r\n\r\n{") {
                        break;
                    }
                }
                _ => break,
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .await
            .ok();
        String::from_utf8_lossy(&raw).to_string()
    };
    let raw = tokio::time::timeout(Duration::from_secs(15), accept)
        .await
        .expect("hook never posted");

    let _ = Command::new("tmux")
        .args(["-L", &socket, "kill-server"])
        .output();

    let (_, body) = raw.split_once("\r\n\r\n").expect("http body");
    let value: serde_json::Value = serde_json::from_str(body.trim()).unwrap();
    assert_eq!(value["session"], "agents");
    assert_eq!(value["state"], "waiting");
    assert_eq!(value["agent"], "claude-code");
    assert!(value["pane"].as_str().unwrap().starts_with('%'));
}
