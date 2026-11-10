//! firefight agent: runs on each client machine, mirrors its assigned timers
//! from the server and executes their actions locally when they finish.

pub mod config;
pub mod exec;
pub mod sound;
pub mod state;

use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use firefight_core::model::{ActionResult, ClientStatus};
use firefight_core::protocol::{AgentMsg, AgentStatus, ServerMsg, UiMsg};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::tungstenite::Message;

use crate::config::AgentConfig;
use crate::exec::Ctx;
use crate::sound::SoundPlayer;
use crate::state::{AgentState, Fire};

#[derive(Clone)]
struct Agent {
    state: Arc<Mutex<AgentState>>,
    config: Arc<Mutex<AgentConfig>>,
    config_path: Option<PathBuf>,
    /// Bumped whenever the UI-visible state changes.
    changed: watch::Sender<u64>,
    sound: SoundPlayer,
    results: mpsc::UnboundedSender<ActionResult>,
    outbox: mpsc::UnboundedSender<AgentMsg>,
}

impl Agent {
    fn state(&self) -> MutexGuard<'_, AgentState> {
        self.state.lock().expect("state mutex poisoned")
    }

    fn notify(&self) {
        self.changed.send_modify(|v| *v += 1);
    }

    fn save_config(&self) {
        let Some(path) = &self.config_path else { return };
        let cfg = self.config.lock().expect("config mutex poisoned").clone();
        if let Err(e) = cfg.save(path) {
            tracing::error!("failed to save {}: {e:#}", path.display());
        }
    }

    fn fire(&self, fire: Fire) {
        tracing::info!(timer = %fire.timer_id, run = %fire.run_id, "timer finished, running actions");
        let ctx = Ctx {
            timer_id: Some(fire.timer_id),
            run_id: Some(fire.run_id),
            allow_loop: fire.allow_loop,
            sound: self.sound.clone(),
            results: self.results.clone(),
        };
        let outbox = self.outbox.clone();
        tokio::spawn(async move {
            exec::execute(fire.actions, ctx).await;
            let _ = outbox.send(AgentMsg::RunComplete { timer_id: fire.timer_id, run_id: fire.run_id });
        });
    }
}

/// Run the agent until the process exits. `ui_listener` serves the local
/// client display; `config_path` (if any) is where credentials are persisted.
pub async fn run(config: AgentConfig, config_path: Option<PathBuf>, ui_listener: TcpListener) -> Result<()> {
    let (changed, _) = watch::channel(0);
    let (results, mut results_rx) = mpsc::unbounded_channel();
    let (outbox, outbox_rx) = mpsc::unbounded_channel();
    let agent = Agent {
        state: Arc::new(Mutex::new(AgentState::new(config.name.clone(), config.client_id))),
        config: Arc::new(Mutex::new(config)),
        config_path,
        changed,
        sound: SoundPlayer::spawn(),
        results,
        outbox,
    };

    // Forward action results to the server (queued while disconnected).
    let fwd = agent.outbox.clone();
    tokio::spawn(async move {
        while let Some(result) = results_rx.recv().await {
            let _ = fwd.send(AgentMsg::ActionResult { result });
        }
    });

    // Local scheduler: fire runs at their deadline even if the server is unreachable.
    let sched = agent.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tick.tick().await;
            let fires = sched.state().due(Utc::now());
            for f in fires {
                sched.fire(f);
            }
        }
    });

    tokio::spawn(serve_ui(agent.clone(), ui_listener));
    connection_loop(agent, outbox_rx).await
}

enum SessionEnd {
    Closed,
    Rejected(String),
}

async fn connection_loop(agent: Agent, mut outbox: mpsc::UnboundedReceiver<AgentMsg>) -> Result<()> {
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        let end = session(&agent, &mut outbox).await;
        agent.state().status = AgentStatus::Disconnected;
        agent.notify();
        match end {
            Ok(SessionEnd::Rejected(reason)) => {
                tracing::warn!("server rejected this agent ({reason}); pairing again");
                {
                    let mut cfg = agent.config.lock().expect("config mutex poisoned");
                    cfg.client_id = None;
                    cfg.token = None;
                }
                agent.state().client_id = None;
                agent.save_config();
                backoff = Duration::from_secs(10);
            }
            Ok(SessionEnd::Closed) => tracing::warn!("disconnected from server"),
            Err(e) => tracing::warn!("connection failed: {e:#}"),
        }
        if started.elapsed() > Duration::from_secs(30) {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(30));
    }
}

async fn session(agent: &Agent, outbox: &mut mpsc::UnboundedReceiver<AgentMsg>) -> Result<SessionEnd> {
    let (url, hello) = {
        let cfg = agent.config.lock().expect("config mutex poisoned");
        let hello = AgentMsg::Hello {
            client_id: cfg.client_id,
            token: cfg.token.clone(),
            hostname: config::hostname(),
            name: cfg.name.clone(),
            enroll_key: cfg.enroll_key.clone(),
        };
        (cfg.agent_ws_url(), hello)
    };
    let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.with_context(|| format!("connecting to {url}"))?;
    ws.send(Message::text(serde_json::to_string(&hello)?)).await?;
    tracing::info!("connected to {url}");

    loop {
        tokio::select! {
            incoming = ws.next() => {
                let text = match incoming {
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(_))) | None => return Ok(SessionEnd::Closed),
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => return Err(e.into()),
                };
                let msg = match serde_json::from_str::<ServerMsg>(&text) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::debug!("bad server message: {e}");
                        continue;
                    }
                };
                if let ServerMsg::Rejected { reason } = msg {
                    return Ok(SessionEnd::Rejected(reason));
                }
                handle_server_msg(agent, msg);
            }
            out = outbox.recv() => {
                let Some(msg) = out else { return Ok(SessionEnd::Closed) };
                ws.send(Message::text(serde_json::to_string(&msg)?)).await?;
            }
        }
    }
}

fn handle_server_msg(agent: &Agent, msg: ServerMsg) {
    let now = Utc::now();
    match msg {
        ServerMsg::Welcome { client_id, status, token } => {
            {
                let mut st = agent.state();
                st.client_id = Some(client_id);
                st.status = match status {
                    ClientStatus::Pending => AgentStatus::Pending,
                    ClientStatus::Approved => AgentStatus::Approved,
                };
            }
            let mut cfg = agent.config.lock().expect("config mutex poisoned");
            let changed = cfg.client_id != Some(client_id) || token.is_some();
            cfg.client_id = Some(client_id);
            if token.is_some() {
                tracing::info!("approved by server; token stored");
                cfg.token = token;
            }
            drop(cfg);
            if changed {
                agent.save_config();
            }
            if status == ClientStatus::Pending {
                tracing::info!(client = %client_id, "waiting for approval in the control deck");
            }
        }
        ServerMsg::Assigned { timers, runs, server_now } => {
            let mut st = agent.state();
            st.observe_clock(server_now, now);
            st.apply_assigned(timers, runs);
        }
        ServerMsg::RunUpdate { run, server_now } => {
            let timer_id = run.timer_id;
            let (fire, stopped) = {
                let mut st = agent.state();
                st.observe_clock(server_now, now);
                st.apply_run(run)
            };
            if stopped {
                agent.sound.stop(timer_id);
            }
            if let Some(f) = fire {
                agent.fire(f);
            }
        }
        ServerMsg::TestRun { actions } => {
            tracing::info!("test run requested");
            // Test runs have no timer to dismiss, so they never loop.
            let ctx = Ctx {
                timer_id: None,
                run_id: None,
                allow_loop: false,
                sound: agent.sound.clone(),
                results: agent.results.clone(),
            };
            tokio::spawn(exec::execute(actions, ctx));
        }
        ServerMsg::Dismissed { timer_id, .. } => agent.sound.stop(timer_id),
        ServerMsg::Rejected { .. } => {}
    }
    agent.notify();
}

/// Serve the local client display over WebSocket (loopback only).
async fn serve_ui(agent: Agent, listener: TcpListener) {
    loop {
        let Ok((stream, _)) = listener.accept().await else { continue };
        let agent = agent.clone();
        tokio::spawn(async move {
            let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else { return };
            let mut changed = agent.changed.subscribe();
            let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
            loop {
                tokio::select! {
                    // Periodic resend keeps the display's clock reference fresh.
                    _ = heartbeat.tick() => {}
                    r = changed.changed() => if r.is_err() { return },
                    incoming = ws.next() => {
                        match incoming {
                            Some(Ok(Message::Text(t))) => {
                                // Release the state lock before acting on the dismiss.
                                let dismiss = match serde_json::from_str(&t) {
                                    Ok(UiMsg::Dismiss { timer_id }) if agent.state().can_dismiss(timer_id) => Some(timer_id),
                                    _ => None,
                                };
                                if let Some(timer_id) = dismiss {
                                    agent.sound.stop(timer_id);
                                    let _ = agent.outbox.send(AgentMsg::Dismiss { timer_id });
                                }
                            }
                            Some(Ok(Message::Close(_))) | None | Some(Err(_)) => return,
                            Some(Ok(_)) => {}
                        }
                        continue;
                    }
                }
                let ev = agent.state().ui_event(Utc::now());
                let json = serde_json::to_string(&ev).expect("serializable");
                if ws.send(Message::text(json)).await.is_err() {
                    return;
                }
            }
        });
    }
}
