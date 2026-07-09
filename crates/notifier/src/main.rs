//! `notifier` — host-side push (DESIGN.md §9).
//!
//!   notifier notify --session agents --pane %3 --state waiting --agent claude-code
//!   notifier watch  --session agents --silence-secs 45
//!
//! `notify` is the tier-1 entrypoint (agent hooks call it); `watch` is the
//! tier-2/3 daemon. Sink selection: --ntfy-url/--ntfy-topic (or NTFY_URL /
//! NTFY_TOPIC env), --fcm-spool for the Phase-9 stub, --stdout for debugging.

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use notifier::{watch, AgentState, FcmStubSink, NtfySink, Payload, Sink, StdoutSink};

#[derive(Parser)]
#[command(
    name = "notifier",
    version,
    about = "HELM push notifier (code-free payloads)"
)]
struct Cli {
    /// ntfy base url (default $NTFY_URL or http://127.0.0.1:2586)
    #[arg(long, global = true)]
    ntfy_url: Option<String>,

    /// ntfy topic (default $NTFY_TOPIC or "helm")
    #[arg(long, global = true)]
    ntfy_topic: Option<String>,

    /// Spool payloads to a file instead (FCM stub for Phase 9)
    #[arg(long, global = true)]
    fcm_spool: Option<std::path::PathBuf>,

    /// Print payloads to stdout instead of pushing
    #[arg(long, global = true)]
    stdout: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Send one notification (tier-1 hooks call this)
    Notify {
        #[arg(long)]
        session: String,
        #[arg(long)]
        pane: String,
        /// waiting | done | error
        #[arg(long, value_parser = parse_state)]
        state: AgentState,
        #[arg(long, default_value = "agent")]
        agent: String,
    },
    /// Watch a session and push on attention/silence (tiers 2+3)
    Watch {
        #[arg(long, default_value = "agents")]
        session: String,
        /// tmux -L socket (default: default server)
        #[arg(long)]
        socket: Option<String>,
        /// Seconds of output silence before a tier-2 push (0 = off)
        #[arg(long, default_value_t = 45)]
        silence_secs: u64,
        /// Suppress duplicate (pane,state) pushes within this window
        #[arg(long, default_value_t = 30)]
        dedupe_secs: u64,
    },
}

fn parse_state(s: &str) -> Result<AgentState, String> {
    s.parse()
}

fn sink_from(cli: &Cli) -> Result<Arc<dyn Sink>> {
    if cli.stdout {
        return Ok(Arc::new(StdoutSink));
    }
    if let Some(spool) = &cli.fcm_spool {
        return Ok(Arc::new(FcmStubSink {
            spool: spool.clone(),
        }));
    }
    let url = cli
        .ntfy_url
        .clone()
        .or_else(|| std::env::var("NTFY_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:2586".to_string());
    let topic = cli
        .ntfy_topic
        .clone()
        .or_else(|| std::env::var("NTFY_TOPIC").ok())
        .unwrap_or_else(|| "helm".to_string());
    Ok(Arc::new(
        NtfySink::from_url(&url, &topic).map_err(|e| anyhow::anyhow!("{e}"))?,
    ))
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let sink = sink_from(&cli)?;

    match &cli.cmd {
        Cmd::Notify {
            session,
            pane,
            state,
            agent,
        } => {
            let payload = Payload::new(session, pane, *state, agent);
            sink.send(&payload)
                .await
                .map_err(|e| anyhow::anyhow!("send via {}: {e}", sink.name()))?;
        }
        Cmd::Watch {
            session,
            socket,
            silence_secs,
            dedupe_secs,
        } => {
            watch::run(
                watch::WatchConfig {
                    socket: socket.clone(),
                    session: session.clone(),
                    silence_secs: *silence_secs,
                    dedupe_secs: *dedupe_secs,
                },
                sink,
            )
            .await
            .context("watch daemon")?;
        }
    }
    Ok(())
}
