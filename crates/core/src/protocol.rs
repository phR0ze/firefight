//! WebSocket messages. All are JSON objects tagged by `"type"`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::model::{Action, ActionResult, ClientId, ClientStatus, ClientView, RunId, TimerDef, TimerId};
use crate::run::TimerRun;

/// Agent -> server.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentMsg {
    /// First message on every connection. `client_id`/`token` are absent until paired.
    Hello {
        client_id: Option<ClientId>,
        token: Option<String>,
        hostname: String,
        name: String,
    },
    ActionResult {
        result: ActionResult,
    },
    /// Dismiss pressed on the client display.
    Dismiss {
        timer_id: TimerId,
    },
    /// Every action of a fired run has finished on this agent.
    RunComplete {
        timer_id: TimerId,
        run_id: RunId,
    },
}

/// Server -> agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Connection accepted. `token` is set only when a new token was just issued
    /// (on approval) and must be persisted by the agent.
    Welcome {
        client_id: ClientId,
        status: ClientStatus,
        token: Option<String>,
    },
    /// Hello was refused (bad token or rejected client). The agent should forget
    /// its credentials and pair again.
    Rejected {
        reason: String,
    },
    /// Full replacement of the timers assigned to this client, with their runs.
    Assigned {
        timers: Vec<TimerDef>,
        runs: Vec<TimerRun>,
        server_now: DateTime<Utc>,
    },
    RunUpdate {
        run: TimerRun,
        server_now: DateTime<Utc>,
    },
    /// Execute actions immediately (the editor's "Test run").
    TestRun {
        actions: Vec<Action>,
    },
    /// Stop looping sounds for a run.
    Dismissed {
        timer_id: TimerId,
        run_id: RunId,
    },
}

/// Server -> control deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminEvent {
    Snapshot { timers: Vec<TimerDef>, runs: Vec<TimerRun>, clients: Vec<ClientView>, server_now: DateTime<Utc> },
    TimerUpdated { timer: TimerDef, run: TimerRun },
    TimerDeleted { id: TimerId },
    RunUpdate { run: TimerRun, server_now: DateTime<Utc> },
    ClientUpdated { client: ClientView },
    ClientDeleted { id: ClientId },
}

/// Connection state of the agent as shown on the client display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Disconnected,
    Pending,
    Approved,
}

/// Agent -> local Flutter client display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiEvent {
    State {
        status: AgentStatus,
        client_id: Option<ClientId>,
        name: String,
        timers: Vec<TimerDef>,
        runs: Vec<TimerRun>,
        /// Server clock (as estimated by the agent) at the time of sending. The UI
        /// compares it to its own clock on receipt to correct `ends_at` for skew.
        server_now: DateTime<Utc>,
    },
}

/// Local Flutter client display -> agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UiMsg {
    Dismiss { timer_id: TimerId },
}
