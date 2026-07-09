//! Desktop harness: headless CLI (Phase 1) + ratatui TUI (Phase 2).
//!
//! The headless commands are the scriptable surface from DESIGN.md §11:
//!   rcoder --transport local list
//!   rcoder --transport local snapshot agents:0.0 --scrollback 200 --ansi
//!   rcoder --transport local send agents:0.0 'y<Enter>'

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use engine::{ConnConfig, Engine, PaneId};
use engine_cli::{render, tui};

#[derive(Parser)]
#[command(
    name = "rcoder",
    version,
    about = "tmux agent remote — desktop harness"
)]
struct Cli {
    /// Transport: local | ssh
    #[arg(long, global = true, default_value = "local")]
    transport: String,

    /// tmux -L socket name (local transport; default: default server)
    #[arg(long, global = true)]
    socket: Option<String>,

    /// Session the TUI scopes to
    #[arg(long, global = true, default_value = "agents")]
    session: String,

    /// SSH host (ssh transport)
    #[arg(long, global = true)]
    host: Option<String>,

    /// SSH port
    #[arg(long, global = true, default_value_t = 22)]
    port: u16,

    /// SSH user (default: $USER)
    #[arg(long, global = true)]
    user: Option<String>,

    /// SSH identity file (ed25519), e.g. .dev/sshd/client_ed25519
    #[arg(long, global = true)]
    key: Option<String>,

    /// Pinned host key fingerprint ("SHA256:…"); omit for trust-on-first-use
    #[arg(long, global = true)]
    hostkey_fp: Option<String>,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List sessions and their panes
    List,
    /// Print a pane grid (plain text, or --ansi for colors)
    Snapshot {
        /// Pane id (%3) or target (agents:0.0)
        target: String,
        /// History lines to include above the visible screen
        #[arg(long, default_value_t = 0)]
        scrollback: u32,
        /// Re-emit colors/attributes as ANSI
        #[arg(long)]
        ansi: bool,
    },
    /// Send keys ("<Name>" convention: 'y<Enter>', '<C-c>', literal text)
    Send { target: String, keys: String },
    /// Interactive TUI (Phase 2)
    Tui,
    /// Host-side: show a pairing QR and enroll one device (§8.3)
    Pair {
        /// Address devices should connect to (e.g. the tailnet IP)
        #[arg(long)]
        pair_host: String,
        /// sshd port devices will use
        #[arg(long, default_value_t = 22)]
        ssh_port: u16,
        /// authorized_keys to append to (default ~/.ssh/authorized_keys)
        #[arg(long)]
        authorized_keys: Option<std::path::PathBuf>,
        /// Installed broker path written into the forced command
        #[arg(long, default_value = "/opt/rcoder/broker")]
        broker_path: String,
        /// Session prefix the broker scopes to
        #[arg(long, default_value = "agents")]
        scope_session: String,
        /// Enroll listener port
        #[arg(long, default_value_t = 7766)]
        enroll_port: u16,
        /// Token lifetime in seconds
        #[arg(long, default_value_t = 600)]
        ttl: u64,
        /// Host public key file for the pinned fingerprint
        #[arg(long)]
        hostkey_pub: Option<std::path::PathBuf>,
    },
    /// Client-side: enroll this machine using the JSON from `rcoder pair`
    Enroll {
        #[arg(long)]
        json: String,
        /// Device name to enroll as
        #[arg(long, default_value = "desktop")]
        device: String,
        /// Key directory (default $XDG_CONFIG_HOME/remote-coder/keys)
        #[arg(long)]
        keys_dir: Option<std::path::PathBuf>,
    },
    /// Host-side: revoke a paired device
    Revoke {
        device: String,
        #[arg(long)]
        authorized_keys: Option<std::path::PathBuf>,
    },
    /// Host-side: list paired devices
    Devices {
        #[arg(long)]
        authorized_keys: Option<std::path::PathBuf>,
    },
}

fn conn_config(cli: &Cli) -> Result<ConnConfig> {
    match cli.transport.as_str() {
        "local" => Ok(ConnConfig::Local {
            socket: cli.socket.clone(),
        }),
        "ssh" => {
            let mut params = engine::SshParams::new(
                cli.host
                    .clone()
                    .context("--host is required for --transport ssh")?,
                cli.port,
                cli.user
                    .clone()
                    .or_else(|| std::env::var("USER").ok())
                    .context("--user is required for --transport ssh")?,
                cli.key
                    .clone()
                    .context("--key <identity file> is required for --transport ssh")?,
            );
            params.hostkey_fp = cli.hostkey_fp.clone();
            if params.hostkey_fp.is_none() {
                eprintln!(
                    "warning: no --hostkey-fp pinned; trusting this connection's host key (TOFU)"
                );
            }
            Ok(ConnConfig::Ssh(params))
        }
        other => bail!("unknown transport {other:?} (expected local|ssh)"),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // §13 log discipline: default WARN; pane bytes only ever at trace.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();

    // Pairing/device management doesn't need (or want) a tmux connection.
    match &cli.cmd {
        Cmd::Pair {
            pair_host,
            ssh_port,
            authorized_keys,
            broker_path,
            scope_session,
            enroll_port,
            ttl,
            hostkey_pub,
        } => {
            return engine_cli::pairing_cmd::pair(
                pair_host.clone(),
                *ssh_port,
                cli.user.clone(),
                authorized_keys.clone(),
                broker_path.clone(),
                scope_session.clone(),
                *enroll_port,
                *ttl,
                hostkey_pub.clone(),
            );
        }
        Cmd::Enroll {
            json,
            device,
            keys_dir,
        } => {
            return engine_cli::pairing_cmd::enroll_cmd(
                json.clone(),
                device.clone(),
                keys_dir.clone(),
            )
            .await;
        }
        Cmd::Revoke {
            device,
            authorized_keys,
        } => return engine_cli::pairing_cmd::revoke(device.clone(), authorized_keys.clone()),
        Cmd::Devices { authorized_keys } => {
            return engine_cli::pairing_cmd::devices(authorized_keys.clone())
        }
        _ => {}
    }

    let engine = Engine::connect(conn_config(&cli)?).await?;

    match &cli.cmd {
        Cmd::List => {
            let sessions = engine.list_sessions().await?;
            if sessions.is_empty() {
                println!("(no tmux sessions)");
                return Ok(());
            }
            for s in sessions {
                println!(
                    "{}  ({} windows, {} client{} attached)",
                    s.name,
                    s.windows,
                    s.attached,
                    if s.attached == 1 { "" } else { "s" }
                );
                // Behind the broker, sessions outside the scope list by name
                // only — their panes are denied host-side (§8.2).
                match engine.list_panes(&s.name).await {
                    Ok(panes) => {
                        for p in panes {
                            println!(
                                "  {}:{}.{}  {}  [{}x{}]  {}{}",
                                p.session,
                                p.window_index,
                                p.pane_index,
                                p.id,
                                p.width,
                                p.height,
                                p.current_command,
                                if p.active && p.window_active {
                                    "  (active)"
                                } else {
                                    ""
                                }
                            );
                        }
                    }
                    Err(e) => println!("  (panes unavailable: {e})"),
                }
            }
        }
        Cmd::Snapshot {
            target,
            scrollback,
            ansi,
        } => {
            let pane = PaneId(target.clone());
            let grid = engine.snapshot(&pane, *scrollback).await?;
            if *ansi {
                print!("{}", render::grid_to_ansi(&grid));
            } else {
                print!("{}", grid.to_text());
            }
        }
        Cmd::Send { target, keys } => {
            let pane = PaneId(target.clone());
            engine.send_key_string(&pane, keys).await?;
        }
        Cmd::Tui => {
            tui::run_tui(engine, cli.session.clone()).await?;
        }
        // Handled before the engine connects.
        Cmd::Pair { .. } | Cmd::Enroll { .. } | Cmd::Revoke { .. } | Cmd::Devices { .. } => {
            unreachable!("pairing commands return early")
        }
    }
    Ok(())
}
