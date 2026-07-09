//! Scripted TUI test (DESIGN.md Phase 2 acceptance): drive the App state
//! machine with synthetic key events against a real tmux server + fake
//! agent, and assert the agent advances. Also renders into a TestBackend
//! to prove the draw path.

use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use engine::{ConnConfig, Engine};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::Terminal;

use engine_cli::tui::{self, App, View};

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
            socket: format!("rc-tui-{hint}-{}", std::process::id()),
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

async fn tick_until<F: Fn(&App) -> bool>(app: &mut App, pred: F, what: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        app.tick().await;
        if pred(app) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}; status={:?} grid=\n{}",
            app.status,
            app.grid.as_ref().map(|g| g.to_text()).unwrap_or_default()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn grid_contains(app: &App, needle: &str) -> bool {
    app.grid
        .as_ref()
        .is_some_and(|g| g.to_text().contains(needle))
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::from(code)
}

#[tokio::test]
async fn tui_app_drives_fake_yn_via_buttons() {
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
    let mut app = App::new(engine, "agents".to_string());

    // List view populates and shows our pane.
    app.refresh_panes().await;
    assert_eq!(app.panes.len(), 1);
    assert_eq!(app.view, View::List);

    // Enter opens the pane view; polling brings the prompt in.
    app.handle_key(key(KeyCode::Enter)).await;
    assert_eq!(app.view, View::Pane);
    tick_until(&mut app, |a| grid_contains(a, "Proceed? (y/n)"), "prompt").await;

    // Render through the real draw path and assert the prompt is on screen.
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| tui::ui::draw(f, &mut app)).unwrap();
    let screen = format!("{:?}", terminal.backend().buffer());
    assert!(screen.contains("Proceed?"), "draw output missing prompt");
    assert!(screen.contains("[F2 Yes]"), "draw output missing buttons");

    // F2 = Yes button → the agent proceeds.
    app.handle_key(key(KeyCode::F(2))).await;
    tick_until(&mut app, |a| grid_contains(a, "proceeding…"), "proceeding").await;

    // Next round: F3 = No button → the agent aborts the step.
    tick_until(
        &mut app,
        |a| grid_contains(a, "Proceed? (y/n)"),
        "second prompt",
    )
    .await;
    // Buttons only exist for the current grid text; ensure fresh prompt shown.
    app.handle_key(key(KeyCode::F(3))).await;
    tick_until(&mut app, |a| grid_contains(a, "step aborted."), "aborted").await;

    // Esc returns to the list.
    app.handle_key(key(KeyCode::Esc)).await;
    assert_eq!(app.view, View::List);
}

#[tokio::test]
async fn tui_input_line_edits_locally_and_esc_clears() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("input");
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

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();
    let mut app = App::new(engine, "agents".to_string());
    app.refresh_panes().await;
    app.handle_key(key(KeyCode::Enter)).await;

    for c in "hello".chars() {
        app.handle_key(key(KeyCode::Char(c))).await;
    }
    assert_eq!(app.input, "hello");
    app.handle_key(key(KeyCode::Backspace)).await;
    assert_eq!(app.input, "hell");

    // First Esc clears input, second leaves the pane view.
    app.handle_key(key(KeyCode::Esc)).await;
    assert_eq!(app.input, "");
    assert_eq!(app.view, View::Pane);
    app.handle_key(key(KeyCode::Esc)).await;
    assert_eq!(app.view, View::List);
}

#[tokio::test]
async fn tui_scroll_offset_pages_and_clamps() {
    if !tmux_available() {
        return;
    }
    let server = TmuxServer::start("scroll");
    server.run(&[
        "new-session",
        "-d",
        "-s",
        "agents",
        "-x",
        "80",
        "-y",
        "10",
        "for i in $(seq 1 40); do echo line-$i; done; sleep 600",
    ]);

    let engine = Engine::connect(ConnConfig::Local {
        socket: Some(server.socket.clone()),
    })
    .await
    .unwrap();
    let mut app = App::new(engine, "agents".to_string());
    app.refresh_panes().await;
    // Viewport must be set before opening the pane: attach() sends it as the
    // control-client size and tmux reflows the window to match.
    app.grid_viewport = (78, 10);
    app.handle_key(key(KeyCode::Enter)).await;
    tick_until(&mut app, |a| grid_contains(a, "line-40"), "tail").await;

    app.handle_key(key(KeyCode::PageUp)).await;
    assert!(app.scroll_offset > 0);
    app.tick().await;
    let (start, end) = app.visible_row_range(10);
    let g = app.grid.as_ref().unwrap();
    let window: Vec<String> = (start..end).map(|r| g.row_text(r)).collect();
    assert!(
        window.iter().any(|l| l.contains("line-")),
        "scrolled window should show history: {window:?}"
    );
    assert!(
        !window.iter().any(|l| l.contains("line-40")),
        "scrolled window should not show the live tail: {window:?}"
    );

    // End returns to live tail.
    app.handle_key(key(KeyCode::End)).await;
    app.tick().await;
    let (start, end) = app.visible_row_range(10);
    let g = app.grid.as_ref().unwrap();
    assert!((start..end).any(|r| g.row_text(r).contains("line-40")));
}
