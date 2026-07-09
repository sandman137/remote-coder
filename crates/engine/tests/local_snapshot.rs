//! Integration tests (DESIGN.md §10.1 layer 3): a real tmux server on a
//! private socket, a fake agent in a pane, driven through `LocalTransport`.
//! Requires tmux on the dev box; set RC_SKIP_TMUX_TESTS=1 to opt out.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use engine::{ConnConfig, Engine, GridSnapshot, KeyInput, PaneId};

fn tmux_available() -> bool {
    if std::env::var_os("RC_SKIP_TMUX_TESTS").is_some() {
        eprintln!("RC_SKIP_TMUX_TESTS set — skipping tmux integration test");
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

/// Private tmux server that dies with the test (even on panic).
struct TmuxServer {
    socket: String,
}

impl TmuxServer {
    fn start(hint: &str) -> Self {
        let socket = format!("rc-test-{hint}-{}", std::process::id());
        Self { socket }
    }

    fn run(&self, args: &[&str]) {
        let status = Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .status()
            .expect("spawn tmux");
        assert!(status.success(), "tmux {args:?} failed");
    }
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .output();
    }
}

async fn wait_for_text(
    engine: &Engine,
    pane: &PaneId,
    needle: &str,
    timeout: Duration,
) -> GridSnapshot {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last = String::new();
    loop {
        if let Ok(grid) = engine.snapshot(pane, 0).await {
            last = grid.to_text();
            if last.contains(needle) {
                return grid;
            }
        }
        if tokio::time::Instant::now() >= deadline {
            panic!("timed out waiting for {needle:?}; last snapshot:\n{last}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn snapshot_and_send_keys_drive_fake_yn() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("yn");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "100",
        "-y",
        "30",
        fixture("fake-yn.sh").to_str().unwrap(),
    ]);

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();

    // Enumeration: the session and its single pane are visible.
    let sessions = engine.list_sessions().await.unwrap();
    assert!(
        sessions.iter().any(|s| s.name == "agents"),
        "sessions: {sessions:?}"
    );
    let panes = engine.list_panes("agents").await.unwrap();
    assert_eq!(panes.len(), 1, "panes: {panes:?}");
    let pane = panes[0].id.clone();
    assert_eq!((panes[0].width, panes[0].height), (100, 30));

    // The fake agent reaches its approval prompt…
    let grid = wait_for_text(&engine, &pane, "Proceed? (y/n)", Duration::from_secs(15)).await;
    assert_eq!((grid.cols, grid.rows), (100, 30));
    // …with the cursor parked right after the prompt on that row.
    let (_, cur_row) = grid.cursor.expect("cursor");
    assert!(
        grid.row_text(cur_row).contains("Proceed? (y/n)"),
        "cursor row {cur_row} should hold the prompt:\n{}",
        grid.to_text()
    );

    // Approving advances it.
    engine
        .send_keys(&pane, &[KeyInput::Text("y".into())])
        .await
        .unwrap();
    wait_for_text(&engine, &pane, "proceeding…", Duration::from_secs(15)).await;

    // Second round: reject via the parsed key-string convenience.
    wait_for_text(&engine, &pane, "Proceed? (y/n)", Duration::from_secs(15)).await;
    engine.send_key_string(&pane, "n").await.unwrap();
    wait_for_text(&engine, &pane, "step aborted.", Duration::from_secs(15)).await;
}

#[tokio::test]
async fn scrollback_snapshot_grows_and_resize_reflows() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("scroll");
    // A shell that emits 60 numbered lines then sleeps: guarantees history.
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "80",
        "-y",
        "10",
        "for i in $(seq 1 60); do echo line-$i; done; sleep 600",
    ]);

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();
    let pane = PaneId("agents:0.0".into());

    wait_for_text(&engine, &pane, "line-60", Duration::from_secs(10)).await;

    let visible = engine.snapshot(&pane, 0).await.unwrap();
    assert_eq!(visible.rows, 10);
    assert!(!visible.to_text().contains("line-1\n"));

    let with_history = engine.snapshot(&pane, 55).await.unwrap();
    assert!(with_history.rows > 10, "rows: {}", with_history.rows);
    assert!(with_history.to_text().contains("line-6\n"));

    // Snapshot-mode reflow: shrinking the window changes the reported grid.
    engine.resize(&pane, 60, 8).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let g = engine.snapshot(&pane, 0).await.unwrap();
        if (g.cols, g.rows) == (60, 8) {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "resize never took effect: {}x{}",
            g.cols,
            g.rows
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn missing_server_yields_empty_sessions() {
    if !tmux_available() {
        return;
    }
    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(format!("rc-test-nosuch-{}", std::process::id())),
    })
    .await
    .unwrap();
    let sessions = engine.list_sessions().await.unwrap();
    assert!(sessions.is_empty());
}
