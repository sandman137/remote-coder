//! The watch daemon: tiers 2 + 3 of attention detection (DESIGN.md §9) with
//! zero agent integration. It drives the engine over LocalTransport,
//! watches every pane of the target session, and pushes a privacy-filtered
//! payload when an agent needs attention and no regular client is looking.
//!
//! Tier 3: the engine's adapter-regex Attention events.
//! Tier 2: output silence — a pane that was streaming output and then went
//! quiet for `silence_secs` is likely waiting on something the regexes
//! don't know; pushed once per quiet period.
//! Tier 1 (agent hooks) calls `notifier notify` directly — same payload
//! path, no daemon involvement.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use engine::{ConnConfig, Engine, EngineEvent};
use tokio::time::Instant;

use crate::{AgentState, Payload, Sink};

pub struct WatchConfig {
    pub socket: Option<String>,
    pub session: String,
    /// Tier-2 silence threshold; 0 disables.
    pub silence_secs: u64,
    /// Suppress duplicate pushes for the same (pane,state) within this window.
    pub dedupe_secs: u64,
}

impl Default for WatchConfig {
    fn default() -> Self {
        WatchConfig {
            socket: None,
            session: "agents".into(),
            silence_secs: 45,
            dedupe_secs: 30,
        }
    }
}

struct PaneActivity {
    last_change: Instant,
    /// Ever produced output since (re)watch — silence only counts after activity.
    active: bool,
    silence_notified: bool,
}

/// Should a push go out, or is a human already looking? "Attached" counts
/// only regular clients — control-mode clients are engines (this daemon,
/// possibly the phone itself; the phone app suppresses foreground pushes
/// client-side, the standard mobile pattern).
async fn regular_client_attached(engine: &Engine, session: &str) -> bool {
    engine
        .list_clients(session)
        .await
        .map(|clients| clients.iter().any(|c| !c.control_mode))
        .unwrap_or(false)
}

pub async fn run(cfg: WatchConfig, sink: Arc<dyn Sink>) -> anyhow::Result<()> {
    let engine = Engine::connect(ConnConfig::Local {
        socket: cfg.socket.clone(),
    })
    .await?;
    let mut events = engine.subscribe();

    // Watch every pane so Grid events flow for activity tracking; attention
    // events fire for all panes regardless.
    let panes = engine.list_panes(&cfg.session).await?;
    if panes.is_empty() {
        anyhow::bail!("session {:?} has no panes", cfg.session);
    }
    for p in &panes {
        engine.attach(&p.id, (p.width, p.height)).await?;
    }
    tracing::info!(session = %cfg.session, panes = panes.len(), sink = sink.name(), "watching");

    let mut activity: HashMap<String, PaneActivity> = HashMap::new();
    let mut last_push: HashMap<(String, AgentState), Instant> = HashMap::new();
    let mut agents: HashMap<String, String> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));

    let push = |pane: &str,
                state: AgentState,
                agent: &str,
                last_push: &mut HashMap<(String, AgentState), Instant>| {
        let key = (pane.to_string(), state);
        if let Some(at) = last_push.get(&key) {
            if at.elapsed() < Duration::from_secs(cfg.dedupe_secs) {
                return None;
            }
        }
        last_push.insert(key, Instant::now());
        Some(Payload::new(&cfg.session, pane, state, agent))
    };

    loop {
        tokio::select! {
            ev = events.recv() => match ev {
                Ok(EngineEvent::Attention { pane, agent, .. }) => {
                    agents.insert(pane.0.clone(), agent.clone());
                    if let Some(track) = activity.get_mut(&pane.0) {
                        track.silence_notified = false;
                    }
                    if !regular_client_attached(&engine, &cfg.session).await {
                        if let Some(p) = push(&pane.0, AgentState::Waiting, &agent, &mut last_push) {
                            deliver(&*sink, &p).await;
                        }
                    }
                }
                Ok(EngineEvent::AttentionCleared { pane }) => {
                    if let Some(track) = activity.get_mut(&pane.0) {
                        track.silence_notified = false;
                    }
                }
                Ok(EngineEvent::Grid { pane, .. }) => {
                    let track = activity.entry(pane.0.clone()).or_insert(PaneActivity {
                        last_change: Instant::now(),
                        active: false,
                        silence_notified: false,
                    });
                    track.last_change = Instant::now();
                    track.active = true;
                    track.silence_notified = false;
                }
                Ok(EngineEvent::Exited { pane, status }) => {
                    let agent = agents.get(&pane.0).cloned().unwrap_or_else(|| "agent".into());
                    let state = match status {
                        Some(0) | None => AgentState::Done,
                        Some(_) => AgentState::Error,
                    };
                    if let Some(p) = push(&pane.0, state, &agent, &mut last_push) {
                        deliver(&*sink, &p).await;
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            },
            _ = tick.tick() => {
                if cfg.silence_secs == 0 {
                    continue;
                }
                for (pane, track) in activity.iter_mut() {
                    if track.active
                        && !track.silence_notified
                        && track.last_change.elapsed() >= Duration::from_secs(cfg.silence_secs)
                    {
                        track.silence_notified = true;
                        let agent = agents.get(pane).cloned().unwrap_or_else(|| "agent".into());
                        if !regular_client_attached(&engine, &cfg.session).await {
                            if let Some(p) = push(pane, AgentState::Waiting, &agent, &mut last_push) {
                                deliver(&*sink, &p).await;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

async fn deliver(sink: &dyn Sink, payload: &Payload) {
    match sink.send(payload).await {
        // Log the content-free title only — never more.
        Ok(()) => tracing::info!(sink = sink.name(), title = %payload.title(), "pushed"),
        Err(e) => tracing::warn!(sink = sink.name(), error = %e, "push failed"),
    }
}
