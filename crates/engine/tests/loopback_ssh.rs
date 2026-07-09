//! Loopback SSH tests (DESIGN.md §10.1 layer 4): `SshTransport` →
//! 127.0.0.1:<port> sshd → tmux, proving auth, host-key pinning, exec, and
//! control-mode-over-SSH on one machine. The Phase-1 (snapshot/send) and
//! Phase-3 (streaming) assertions rerun unchanged over the SSH transport.
//!
//! Needs /usr/sbin/sshd + ssh-keygen; set HELM_SKIP_SSH_TESTS=1 to opt out.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use engine::{ConnConfig, Engine, EngineEvent, EventStream, PaneId, SshParams, TransportError};

const SSHD_BIN: &str = "/usr/sbin/sshd";

fn ssh_tests_enabled() -> bool {
    if std::env::var_os("HELM_SKIP_SSH_TESTS").is_some() {
        return false;
    }
    if !Path::new(SSHD_BIN).exists() {
        eprintln!("{SSHD_BIN} not found — skipping loopback SSH tests");
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

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn keygen(path: &Path, comment: &str) {
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", comment, "-f"])
        .arg(path)
        .status()
        .expect("ssh-keygen");
    assert!(status.success(), "ssh-keygen failed");
}

fn host_fingerprint(pub_path: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .args(["-l", "-f"])
        .arg(pub_path)
        .output()
        .expect("ssh-keygen -l");
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .expect("fingerprint field")
        .to_string()
}

/// Loopback sshd + private tmux server. The client key is installed with a
/// forced command that rewrites `tmux …` to `tmux -L <test socket> …`, so
/// remote invocations land on the private server (and the forced-command
/// flow itself — Phase 6's foundation — is exercised).
struct SshFixture {
    dir: PathBuf,
    port: u16,
    tmux_socket: String,
    sshd: Child,
    host_fp: String,
}

impl SshFixture {
    fn start(hint: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("helm-ssh-{hint}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tmux_socket = format!("helm-ssh-{hint}-{}", std::process::id());
        let port = free_port();

        let host_key = dir.join("host_ed25519");
        let client_key = dir.join("client_ed25519");
        keygen(&host_key, "helm-test-host");
        keygen(&client_key, "helm-test-client");
        let host_fp = host_fingerprint(&dir.join("host_ed25519.pub"));

        // Forced command: rewrite a leading "tmux" into "tmux -L <socket>".
        let wrapper = dir.join("tmux-wrapper.sh");
        std::fs::write(
            &wrapper,
            format!(
                "#!/usr/bin/env bash\n\
                 set -eu\n\
                 cmd=\"${{SSH_ORIGINAL_COMMAND:-}}\"\n\
                 case \"$cmd\" in\n\
                   tmux\\ *) eval \"exec tmux -L {tmux_socket} ${{cmd#tmux }}\" ;;\n\
                   tmux)    exec tmux -L {tmux_socket} ;;\n\
                   *)       echo \"denied: $cmd\" >&2; exit 66 ;;\n\
                 esac\n"
            ),
        )
        .unwrap();
        set_exec(&wrapper);

        let pubkey = std::fs::read_to_string(dir.join("client_ed25519.pub")).unwrap();
        std::fs::write(
            dir.join("authorized_keys"),
            format!(
                "command=\"{}\",no-port-forwarding,no-x11-forwarding,no-agent-forwarding {}",
                wrapper.display(),
                pubkey
            ),
        )
        .unwrap();
        set_600(&dir.join("authorized_keys"));

        std::fs::write(
            dir.join("sshd_config"),
            format!(
                "Port {port}\n\
                 ListenAddress 127.0.0.1\n\
                 HostKey {host}\n\
                 PidFile {pid}\n\
                 AuthorizedKeysFile {auth}\n\
                 PasswordAuthentication no\n\
                 KbdInteractiveAuthentication no\n\
                 PubkeyAuthentication yes\n\
                 StrictModes no\n\
                 UsePAM no\n\
                 LogLevel ERROR\n",
                host = dir.join("host_ed25519").display(),
                pid = dir.join("sshd.pid").display(),
                auth = dir.join("authorized_keys").display(),
            ),
        )
        .unwrap();

        let sshd = Command::new(SSHD_BIN)
            .args(["-D", "-e", "-f"])
            .arg(dir.join("sshd_config"))
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn sshd");

        // Wait for the listener.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "sshd never started listening on {port}"
            );
            std::thread::sleep(Duration::from_millis(100));
        }

        SshFixture {
            dir,
            port,
            tmux_socket,
            sshd,
            host_fp,
        }
    }

    fn tmux(&self, args: &[&str]) {
        let status = Command::new("tmux")
            .args(["-L", &self.tmux_socket, "-f", "/dev/null"])
            .args(args)
            .status()
            .expect("spawn tmux");
        assert!(status.success(), "tmux {args:?} failed");
    }

    fn params(&self) -> SshParams {
        let mut p = SshParams::new(
            "127.0.0.1",
            self.port,
            std::env::var("USER").expect("$USER"),
            self.dir.join("client_ed25519").display().to_string(),
        );
        p.hostkey_fp = Some(self.host_fp.clone());
        p
    }
}

fn set_exec(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn set_600(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).unwrap();
}

impl Drop for SshFixture {
    fn drop(&mut self) {
        let _ = self.sshd.kill();
        let _ = self.sshd.wait();
        let _ = Command::new("tmux")
            .args(["-L", &self.tmux_socket, "kill-server"])
            .output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

async fn wait_for_text(engine: &Engine, pane: &PaneId, needle: &str, secs: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(secs);
    let mut last = String::new();
    loop {
        if let Ok(grid) = engine.snapshot(pane, 0).await {
            last = grid.to_text();
            if last.contains(needle) {
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {needle:?} over ssh; last:\n{last}"
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
}

#[tokio::test]
async fn phase1_assertions_over_ssh() {
    if !ssh_tests_enabled() {
        return;
    }
    let fx = SshFixture::start("p1");
    fx.tmux(&[
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

    let engine = Engine::connect(ConnConfig::Ssh(fx.params())).await.unwrap();

    let sessions = engine.list_sessions().await.unwrap();
    assert!(sessions.iter().any(|s| s.name == "agents"));
    let panes = engine.list_panes("agents").await.unwrap();
    assert_eq!(panes.len(), 1);
    let pane = panes[0].id.clone();

    wait_for_text(&engine, &pane, "Proceed? (y/n)", 20).await;
    engine.send_key_string(&pane, "y").await.unwrap();
    wait_for_text(&engine, &pane, "proceeding…", 20).await;
}

#[tokio::test]
async fn phase3_streaming_over_ssh() {
    if !ssh_tests_enabled() {
        return;
    }
    let fx = SshFixture::start("p3");
    fx.tmux(&[
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
    fx.tmux(&["set-option", "-g", "window-size", "latest"]);

    let engine = Engine::connect(ConnConfig::Ssh(fx.params())).await.unwrap();
    let pane = PaneId("%0".into());
    let mut events: EventStream = engine.subscribe();
    engine.attach(&pane, (60, 18)).await.unwrap();

    // Streaming + reflow to the client size, all over the SSH channel.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut saw_tick = false;
    let mut saw_reflow = false;
    while tokio::time::Instant::now() < deadline && !(saw_tick && saw_reflow) {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Ok(EngineEvent::Grid { snapshot, .. })) => {
                if snapshot.to_text().contains("stream tick") {
                    saw_tick = true;
                }
                if (snapshot.cols, snapshot.rows) == (60, 18) {
                    saw_reflow = true;
                }
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            _ => break,
        }
    }
    assert!(
        saw_tick && saw_reflow,
        "streaming over ssh: tick={saw_tick} reflow={saw_reflow}"
    );
}

#[tokio::test]
async fn pinning_rejects_wrong_host_key() {
    if !ssh_tests_enabled() {
        return;
    }
    let fx = SshFixture::start("pin");

    let mut params = fx.params();
    params.hostkey_fp = Some("SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into());
    match Engine::connect(ConnConfig::Ssh(params)).await {
        Err(engine::EngineError::Transport(TransportError::HostKeyMismatch {
            pinned,
            presented,
        })) => {
            assert!(pinned.starts_with("SHA256:AAAA"));
            assert_eq!(presented, fx.host_fp);
        }
        Err(other) => panic!("expected HostKeyMismatch, got {other:?}"),
        Ok(_) => panic!("connection with wrong pin must fail"),
    }

    // Correct pin connects fine.
    if let Err(e) = Engine::connect(ConnConfig::Ssh(fx.params())).await {
        panic!("correct pin rejected: {e}");
    }
}

#[tokio::test]
async fn forced_command_denies_non_tmux() {
    if !ssh_tests_enabled() {
        return;
    }
    let fx = SshFixture::start("deny");
    // Reach through the transport directly with a non-tmux prefix: the
    // wrapper must refuse it (exit 66) — nothing but tmux goes through.
    let mut params = fx.params();
    params.tmux_prefix = vec!["id".into()];
    let engine = Engine::connect(ConnConfig::Ssh(params)).await.unwrap();
    let err = engine.list_sessions().await.unwrap_err();
    match err {
        engine::EngineError::Transport(TransportError::Tmux { status, stderr }) => {
            assert_eq!(status, 66);
            assert!(stderr.contains("denied"), "stderr: {stderr}");
        }
        other => panic!("expected denial, got {other:?}"),
    }
}
