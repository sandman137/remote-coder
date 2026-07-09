//! Desktop harness: headless CLI (Phase 1) + ratatui TUI (Phase 2).
//!
//! The headless commands are the scriptable surface from DESIGN.md §11:
//!   helm --transport local list
//!   helm --transport local snapshot agents:0.0 --scrollback 200 --ansi
//!   helm --transport local send agents:0.0 'y<Enter>'

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use engine::{ConnConfig, Engine, PaneId};
use engine_cli::{render, tui};

#[derive(Parser)]
#[command(name = "helm", version, about = "tmux agent remote — desktop harness")]
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
}

fn conn_config(cli: &Cli) -> Result<ConnConfig> {
    match cli.transport.as_str() {
        "local" => Ok(ConnConfig::Local {
            socket: cli.socket.clone(),
        }),
        "ssh" => Ok(ConnConfig::Ssh {
            host: cli
                .host
                .clone()
                .context("--host is required for --transport ssh")?,
            port: cli.port,
            user: cli
                .user
                .clone()
                .or_else(|| std::env::var("USER").ok())
                .context("--user is required for --transport ssh")?,
            key_path: None,
            hostkey_fp: None,
        }),
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
                for p in engine.list_panes(&s.name).await? {
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
    }
    Ok(())
}
