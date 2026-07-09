//! SSH forced-command entrypoint (DESIGN.md §8.2). Installed as:
//!
//! ```text
//! command="/opt/rcoder/broker --session agents",no-port-forwarding,... ssh-ed25519 AAAA… rcoder:<device>
//! ```
//!
//! Reads `$SSH_ORIGINAL_COMMAND`, authorizes it against the whitelist +
//! session scope, and execs tmux (argv array — never a shell). Denials log
//! to stderr and exit 65.

use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};

use broker::{authorize, Decision, PaneResolver};

const DENY_EXIT: i32 = 65;

struct Opts {
    session: String,
    tmux_bin: String,
    tmux_socket: Option<String>,
}

fn parse_opts() -> Opts {
    let mut opts = Opts {
        session: "agents".to_string(),
        tmux_bin: "tmux".to_string(),
        tmux_socket: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().unwrap_or_else(|| usage());
        match a.as_str() {
            "--session" => opts.session = value(),
            "--tmux-bin" => opts.tmux_bin = value(),
            "--tmux-socket" => opts.tmux_socket = Some(value()),
            _ => usage(),
        }
    }
    opts
}

fn usage() -> ! {
    eprintln!("usage: broker [--session <name>] [--tmux-bin <path>] [--tmux-socket <name>]");
    std::process::exit(DENY_EXIT);
}

/// Resolves %pane targets by asking tmux which session owns them.
struct TmuxResolver<'a> {
    opts: &'a Opts,
}

impl PaneResolver for TmuxResolver<'_> {
    fn session_of_pane(&self, pane_id: &str) -> Option<String> {
        let out = base_command(self.opts)
            .args(["list-panes", "-a", "-F", "#{pane_id}\u{1f}#{session_name}"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        // Output is vis-escaped; our separator arrives as literal \037.
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Some((id, session)) = line.split_once("\\037") {
                if id == pane_id {
                    return Some(session.to_string());
                }
            }
        }
        None
    }
}

fn base_command(opts: &Opts) -> Command {
    let mut cmd = Command::new(&opts.tmux_bin);
    if let Some(sock) = &opts.tmux_socket {
        cmd.args(["-L", sock]);
    }
    cmd
}

/// Receive `size` bytes on stdin and store them under `~/.rcoder/uploads/`
/// (0700 dir, 0600 file, epoch-prefixed name — never overwrites). Prints the
/// absolute path on stdout; the client inserts it into the agent's prompt.
fn receive_upload(name: &str, size: u64) -> Result<std::path::PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let dir = std::path::Path::new(&home).join(".rcoder").join("uploads");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let path = dir.join(format!("{stamp}-{name}"));

    let mut buf = vec![0u8; size as usize];
    std::io::stdin()
        .read_exact(&mut buf)
        .map_err(|e| format!("short read ({e})"))?;

    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .map_err(|e| e.to_string())?;
    f.write_all(&buf).map_err(|e| e.to_string())?;
    Ok(path)
}

/// Self-heal: recreate the scoped session if it died (tmux kills a session
/// with its last window). A paired phone should land in an empty session and
/// see "no panes yet" — never a hard connection error.
fn ensure_session(opts: &Opts) {
    let exists = base_command(opts)
        .args(["has-session", "-t", &format!("={}", opts.session)])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !exists {
        let _ = base_command(opts)
            .args(["new-session", "-d", "-s", &opts.session, "-x", "220", "-y", "50"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn main() {
    let opts = parse_opts();
    let original = match std::env::var("SSH_ORIGINAL_COMMAND") {
        Ok(v) => v,
        Err(_) => {
            eprintln!("rc-broker: no command supplied (interactive shells are not brokered)");
            std::process::exit(DENY_EXIT);
        }
    };

    let resolver = TmuxResolver { opts: &opts };
    match authorize(&original, &opts.session, &resolver) {
        Decision::Allowed(argv) => {
            ensure_session(&opts);
            // exec replaces this process; stdio passes straight through, so
            // control-mode streaming works unchanged behind the broker.
            let err = base_command(&opts).args(&argv).exec();
            eprintln!("rc-broker: exec tmux failed: {err}");
            std::process::exit(DENY_EXIT);
        }
        Decision::Upload { name, size } => match receive_upload(&name, size) {
            Ok(path) => {
                println!("{}", path.display());
                std::process::exit(0);
            }
            Err(e) => {
                eprintln!("rc-broker: upload failed: {e}");
                std::process::exit(DENY_EXIT);
            }
        },
        Decision::Denied(reason) => {
            // Reason only — never echo pane content or key material.
            eprintln!("rc-broker: denied: {reason}");
            std::process::exit(DENY_EXIT);
        }
    }
}
