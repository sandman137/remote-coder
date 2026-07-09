//! Phase 6 acceptance: end-to-end loopback with the REAL broker binary as
//! the forced command — allowed commands succeed, out-of-scope and
//! non-whitelisted commands fail, and control-mode streaming works through
//! the broker's exec.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use engine::{ConnConfig, Engine, EngineEvent, PaneId, SshParams};

const SSHD_BIN: &str = "/usr/sbin/sshd";

fn enabled() -> bool {
    if std::env::var_os("HELM_SKIP_SSH_TESTS").is_some() {
        return false;
    }
    Path::new(SSHD_BIN).exists()
        && Command::new("tmux")
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

fn keygen(path: &Path) {
    assert!(Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f"])
        .arg(path)
        .status()
        .unwrap()
        .success());
}

fn fingerprint(pub_path: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .args(["-l", "-f"])
        .arg(pub_path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string()
}

struct BrokeredSsh {
    dir: PathBuf,
    port: u16,
    tmux_socket: String,
    sshd: Child,
    host_fp: String,
}

impl BrokeredSsh {
    fn start(hint: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("helm-brk-{hint}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tmux_socket = format!("helm-brk-{hint}-{}", std::process::id());
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();

        keygen(&dir.join("host_ed25519"));
        keygen(&dir.join("client_ed25519"));
        let host_fp = fingerprint(&dir.join("host_ed25519.pub"));

        // The actual broker binary as forced command, scoped to "agents" on
        // the private test server — installed exactly as production would,
        // via the enroll helper.
        let broker_cmd = format!(
            "{} --session agents --tmux-socket {tmux_socket}",
            env!("CARGO_BIN_EXE_broker")
        );
        let pubkey = std::fs::read_to_string(dir.join("client_ed25519.pub")).unwrap();
        broker::enroll::add_authorized_key(
            &dir.join("authorized_keys"),
            "test-device",
            &pubkey,
            &broker_cmd,
        )
        .unwrap();

        std::fs::write(
            dir.join("sshd_config"),
            format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {host}\nPidFile {pid}\n\
                 AuthorizedKeysFile {auth}\nPasswordAuthentication no\n\
                 KbdInteractiveAuthentication no\nPubkeyAuthentication yes\nStrictModes no\n\
                 UsePAM no\nLogLevel ERROR\n",
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
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(std::time::Instant::now() < deadline, "sshd never listened");
            std::thread::sleep(Duration::from_millis(100));
        }

        BrokeredSsh {
            dir,
            port,
            tmux_socket,
            sshd,
            host_fp,
        }
    }

    fn tmux(&self, args: &[&str]) {
        assert!(Command::new("tmux")
            .args(["-L", &self.tmux_socket, "-f", "/dev/null"])
            .args(args)
            .status()
            .unwrap()
            .success());
    }

    fn params(&self) -> SshParams {
        let mut p = SshParams::new(
            "127.0.0.1",
            self.port,
            std::env::var("USER").unwrap(),
            self.dir.join("client_ed25519").display().to_string(),
        );
        p.hostkey_fp = Some(self.host_fp.clone());
        p
    }
}

impl Drop for BrokeredSsh {
    fn drop(&mut self) {
        let _ = self.sshd.kill();
        let _ = self.sshd.wait();
        let _ = Command::new("tmux")
            .args(["-L", &self.tmux_socket, "kill-server"])
            .output();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[tokio::test]
async fn brokered_session_full_flow_and_scoping() {
    if !enabled() {
        return;
    }
    let fx = BrokeredSsh::start("flow");
    fx.tmux(&[
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
    // A second session that must stay invisible to the brokered device.
    fx.tmux(&[
        "new-session",
        "-d",
        "-s",
        "secret",
        "-x",
        "80",
        "-y",
        "24",
        "sleep 600",
    ]);

    let engine = Engine::connect(ConnConfig::Ssh(fx.params())).await.unwrap();

    // Scoped enumeration works.
    let panes = engine.list_panes("agents").await.unwrap();
    assert_eq!(panes.len(), 1);
    let pane = panes[0].id.clone();

    // Snapshot + send-keys through the broker.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let g = engine.snapshot(&pane, 0).await.unwrap();
        if g.to_text().contains("Proceed? (y/n)") {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "no prompt");
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    engine.send_key_string(&pane, "y").await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    loop {
        let g = engine.snapshot(&pane, 0).await.unwrap();
        if g.to_text().contains("proceeding…") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "send-keys never landed"
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
    }

    // Control-mode streaming through the broker's exec.
    let mut events = engine.subscribe();
    engine.attach(&pane, (70, 20)).await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut streamed = false;
    while tokio::time::Instant::now() < deadline && !streamed {
        match tokio::time::timeout(Duration::from_secs(5), events.recv()).await {
            Ok(Ok(EngineEvent::Grid { snapshot, .. })) => {
                streamed = snapshot.to_text().contains("Proceed?")
                    || snapshot.to_text().contains("working on task");
            }
            Ok(Ok(_)) => {}
            Ok(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
            _ => break,
        }
    }
    assert!(streamed, "no grid events through the broker");

    // Out-of-scope: the secret session is unreachable in every shape.
    assert!(engine.list_panes("secret").await.is_err());
    assert!(engine
        .snapshot(&PaneId("secret:0.0".into()), 0)
        .await
        .is_err());
    assert!(engine
        .send_key_string(&PaneId("secret:0.0".into()), "id<Enter>")
        .await
        .is_err());
}

#[tokio::test]
async fn broker_denies_shell_and_unlisted_commands() {
    if !enabled() {
        return;
    }
    let fx = BrokeredSsh::start("deny");
    fx.tmux(&["new-session", "-d", "-s", "agents", "sleep 600"]);

    // Non-tmux command line (transport-level prefix override): the broker
    // must refuse to run anything but tmux. (Per-flag denials like
    // list-panes -a are unit-tested against authorize() in lib.rs.)
    let mut params = fx.params();
    params.tmux_prefix = vec!["bash".into(), "-i".into()];
    let engine = Engine::connect(ConnConfig::Ssh(params)).await.unwrap();
    match engine.list_sessions().await {
        Err(engine::EngineError::Transport(engine::TransportError::Tmux { status, stderr })) => {
            assert_eq!(status, 65);
            assert!(stderr.contains("denied"), "stderr: {stderr}");
        }
        other => panic!("expected broker denial, got {other:?}"),
    }
}
