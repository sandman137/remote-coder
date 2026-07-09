//! Phase 4 acceptance: live attention events + button sets from the fake
//! agents, metadata extraction, press_button, and launch_agent — all over a
//! real tmux server + control-mode streaming.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use engine::{ConnConfig, Engine, EngineEvent, EventStream, PaneId, PromptKind};

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
            socket: format!("rc-attn-{hint}-{}", std::process::id()),
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
}

impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .output();
    }
}

async fn wait_event<F, T>(events: &mut EventStream, what: &str, mut pick: F) -> T
where
    F: FnMut(EngineEvent) -> Option<T>,
{
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now()) else {
            panic!("timed out waiting for {what}");
        };
        match tokio::time::timeout(remaining, events.recv()).await {
            Ok(Ok(ev)) => {
                if let Some(out) = pick(ev) {
                    return out;
                }
            }
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            Ok(Err(e)) => panic!("event stream closed waiting for {what}: {e}"),
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

#[tokio::test]
async fn yn_agent_fires_attention_with_buttons_and_metadata() {
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
        "90",
        "-y",
        "28",
        fixture("fake-yn.sh").to_str().unwrap(),
    ]);

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();
    let pane = PaneId("%0".into());
    let mut events = engine.subscribe();
    engine.attach(&pane, (90, 28)).await.unwrap();

    // The fake agent reaches its prompt → Attention with the adapter's set.
    let (agent, kind, buttons) = wait_event(&mut events, "attention", |ev| match ev {
        EngineEvent::Attention {
            agent,
            kind,
            buttons,
            ..
        } => Some((agent, kind, buttons)),
        _ => None,
    })
    .await;
    assert_eq!(agent, "fake-yn");
    assert_eq!(kind, PromptKind::YesNo);
    let labels: Vec<&str> = buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, vec!["Yes", "No"]);

    // Approving via the adapter button: attention clears while the agent
    // works, the next round's metadata (tokens changed) fires, and the new
    // prompt re-raises attention.
    engine.press_button(&pane, "Yes").await.unwrap();
    let (mut cleared, mut metadata, mut re_raised) = (false, false, false);
    wait_event(&mut events, "cleared + metadata + re-raise", |ev| {
        match ev {
            EngineEvent::AttentionCleared { .. } => cleared = true,
            EngineEvent::Metadata { fields, .. } => {
                if fields.contains_key("tokens") {
                    metadata = true;
                }
            }
            EngineEvent::Attention { agent, .. } if agent == "fake-yn" && cleared => {
                re_raised = true;
            }
            _ => {}
        }
        (cleared && metadata && re_raised).then_some(())
    })
    .await;
}

#[tokio::test]
async fn numbered_agent_classifies_menu_and_applies() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("menu");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "90",
        "-y",
        "28",
        fixture("fake-numbered.sh").to_str().unwrap(),
    ]);

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();
    let pane = PaneId("%0".into());
    let mut events = engine.subscribe();
    engine.attach(&pane, (90, 28)).await.unwrap();

    let (agent, kind, buttons) = wait_event(&mut events, "menu attention", |ev| match ev {
        EngineEvent::Attention {
            agent,
            kind,
            buttons,
            ..
        } => Some((agent, kind, buttons)),
        _ => None,
    })
    .await;
    assert_eq!(agent, "fake-numbered");
    assert_eq!(kind, PromptKind::Menu);
    let labels: Vec<&str> = buttons.iter().map(|b| b.label.as_str()).collect();
    assert_eq!(labels, vec!["Apply", "Skip", "Abort"]);

    engine.press_button(&pane, "Apply").await.unwrap();
    // "applied." lands in the grid.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let grid = engine.snapshot(&pane, 0).await.unwrap();
        if grid.to_text().contains("applied.") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "apply never landed:\n{}",
            grid.to_text()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn launch_agent_creates_window_running_adapter_cmd() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("launch");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "80",
        "-y",
        "24",
        "sleep 600",
    ]);

    // Override registry: a launcher adapter pointing at the fixture by
    // absolute path — also exercises Registry DI + user-override loading.
    let dir = std::env::temp_dir().join(format!("rc-launch-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("launchable.toml"),
        format!(
            r#"
id = "launchable"
name = "Launchable Fake"
launch = {{ cmd = "{}" }}
attention = ['Proceed\? \(y/n\)']
"#,
            fixture("fake-yn.sh").display()
        ),
    )
    .unwrap();
    let registry = engine::Registry::load_with_overrides(Some(&dir)).unwrap();
    let engine = Engine::connect_with_registry(
        ConnConfig::Local {
            socket: Some(server.socket.clone()),
        },
        registry,
    )
    .await
    .unwrap();

    let pane = engine
        .launch_agent("agents", "launchable", Some("/tmp".into()))
        .await
        .unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert!(pane.as_str().starts_with('%'), "pane id: {pane}");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let grid = engine.snapshot(&pane, 0).await.unwrap();
        if grid.to_text().contains("Proceed? (y/n)") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "launched agent never prompted:\n{}",
            grid.to_text()
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    // Unknown adapter errors cleanly.
    assert!(engine
        .launch_agent("agents", "no-such-agent", None)
        .await
        .is_err());
}
