//! Streaming integration (DESIGN.md Phase 3 acceptance): attach to a live
//! fake agent over control mode, observe Grid events, reflow the pane by
//! client size, and survive a client detach (reconnect path).

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use engine::{Color, ConnConfig, Engine, EngineEvent, EventStream, GridSnapshot, PaneId};

fn tmux_available() -> bool {
    if std::env::var_os("HELM_SKIP_TMUX_TESTS").is_some() {
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
            socket: format!("helm-stream-{hint}-{}", std::process::id()),
        }
    }
    fn run(&self, args: &[&str]) {
        let status = Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .status()
            .expect("spawn tmux");
        assert!(status.success(), "tmux {args:?} failed");
    }

    fn run_out(&self, args: &[&str]) -> String {
        let out = Command::new("tmux")
            .args(["-L", &self.socket, "-f", "/dev/null"])
            .args(args)
            .output()
            .expect("spawn tmux");
        assert!(out.status.success(), "tmux {args:?} failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .output();
    }
}

/// Wait for a Grid event matching `pred`.
async fn next_grid<F: Fn(&GridSnapshot) -> bool>(
    events: &mut EventStream,
    pred: F,
    what: &str,
    timeout: Duration,
) -> GridSnapshot {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut last: Option<GridSnapshot> = None;
    loop {
        let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) else {
            panic!(
                "timed out waiting for {what}; last grid:\n{}",
                last.as_ref().map(|g| g.to_text()).unwrap_or_default()
            );
        };
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(EngineEvent::Grid { snapshot, .. })) => {
                if pred(&snapshot) {
                    return snapshot;
                }
                last = Some(snapshot);
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(e)) => panic!("event stream closed waiting for {what}: {e}"),
            Err(_) => panic!(
                "timed out waiting for {what}; last grid:\n{}",
                last.as_ref().map(|g| g.to_text()).unwrap_or_default()
            ),
        }
    }
}

#[tokio::test]
async fn streaming_grid_events_flow_and_carry_color() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("flow");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "80",
        "-y",
        "24",
        fixture("fake-stream.sh").to_str().unwrap(),
    ]);
    server.run(&["set-option", "-g", "window-size", "latest"]);

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();
    let pane = PaneId("%0".into());
    let mut events = engine.subscribe();
    engine.attach(&pane, (80, 24)).await.unwrap();

    // Two successive ticks prove live streaming (not just the primed grid).
    let first = next_grid(
        &mut events,
        |g| g.to_text().contains("stream tick"),
        "first tick",
        Duration::from_secs(15),
    )
    .await;
    let first_text = first.to_text();
    next_grid(
        &mut events,
        |g| g.to_text().contains("stream tick") && g.to_text() != first_text,
        "second, different tick",
        Duration::from_secs(15),
    )
    .await;

    // Colors must flow through the VT path (256-color tick lines).
    let colored = next_grid(
        &mut events,
        |g| {
            g.cells
                .iter()
                .any(|c| matches!(c.fg, Color::Indexed(n) if n >= 16))
        },
        "a 256-colored cell",
        Duration::from_secs(15),
    )
    .await;
    assert!(colored.cells.iter().any(|c| c.fg != Color::Default));
}

#[tokio::test]
async fn client_size_reflows_pane() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("reflow");
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
    server.run(&["set-option", "-g", "window-size", "latest"]);

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();
    let pane = PaneId("%0".into());
    let mut events = engine.subscribe();

    // Attach at phone-ish size: tmux must reflow the window for us (§4.3).
    engine.attach(&pane, (60, 15)).await.unwrap();
    next_grid(
        &mut events,
        |g| (g.cols, g.rows) == (60, 15),
        "grid at 60x15",
        Duration::from_secs(15),
    )
    .await;
    let panes = engine.list_panes("agents").await.unwrap();
    assert_eq!((panes[0].width, panes[0].height), (60, 15));

    // Rotate the phone: resize() while streaming goes through refresh-client.
    engine.resize(&pane, 90, 20).await.unwrap();
    next_grid(
        &mut events,
        |g| (g.cols, g.rows) == (90, 20),
        "grid at 90x20",
        Duration::from_secs(15),
    )
    .await;
}

#[tokio::test]
async fn reconnects_after_client_detach() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("reconn");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "80",
        "-y",
        "24",
        fixture("fake-stream.sh").to_str().unwrap(),
    ]);

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();
    let pane = PaneId("%0".into());
    let mut events = engine.subscribe();
    engine.attach(&pane, (80, 24)).await.unwrap();

    next_grid(
        &mut events,
        |g| g.to_text().contains("stream tick"),
        "pre-detach tick",
        Duration::from_secs(15),
    )
    .await;

    // Detach the control client by name (detach-client -a skips control
    // clients). It receives %exit; the streamer must reconnect.
    let client = server.run_out(&["list-clients", "-F", "#{client_name}"]);
    let client = client.lines().next().expect("a control client attached");
    server.run(&["detach-client", "-t", client]);

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let (mut saw_reconnecting, mut saw_connected) = (false, false);
    while tokio::time::Instant::now() < deadline && !(saw_reconnecting && saw_connected) {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Ok(EngineEvent::Reconnecting)) => saw_reconnecting = true,
            Ok(Ok(EngineEvent::Connected)) if saw_reconnecting => saw_connected = true,
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            _ => break,
        }
    }
    assert!(
        saw_reconnecting && saw_connected,
        "expected Reconnecting → Connected (got reconnecting={saw_reconnecting} connected={saw_connected})"
    );

    // Streaming resumed.
    next_grid(
        &mut events,
        |g| g.to_text().contains("stream tick"),
        "post-reconnect tick",
        Duration::from_secs(15),
    )
    .await;
}
