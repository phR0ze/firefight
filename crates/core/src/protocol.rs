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
        /// Shared enrollment key; when it matches the server's, the client is approved
        /// without a manual step in the control deck.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        enroll_key: Option<String>,
    },
    ActionResult {
        result: ActionResult,
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
    /// The run was reset or dismissed: stop its actions and sounds if still going.
    Dismissed {
        timer_id: TimerId,
        run_id: RunId,
    },
}

/// Server -> control deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminEvent {
    Snapshot {
        timers: Vec<TimerDef>,
        /// Shared runs, followed by every client without its own copy.
        runs: Vec<TimerRun>,
        /// Copies of a run that individual clients follow instead of the shared one.
        client_runs: Vec<ClientRun>,
        clients: Vec<ClientView>,
        server_now: DateTime<Utc>,
    },
    TimerUpdated {
        timer: TimerDef,
        run: TimerRun,
    },
    TimerDeleted {
        id: TimerId,
    },
    RunUpdate {
        run: TimerRun,
        server_now: DateTime<Utc>,
    },
    /// A client's own copy of a timer's run changed.
    ClientRunUpdate {
        client_id: ClientId,
        run: TimerRun,
        server_now: DateTime<Utc>,
    },
    /// A client dropped its own copy and follows the shared run again.
    ClientRunRemoved {
        client_id: ClientId,
        timer_id: TimerId,
    },
    ClientUpdated {
        client: ClientView,
    },
    ClientDeleted {
        id: ClientId,
    },
}

/// A run followed by one client only, started from its card in the control deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientRun {
    pub client_id: ClientId,
    pub run: TimerRun,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_enroll_key_is_optional_on_the_wire() {
        // Agents without the field still parse, and it's omitted when unset.
        let old = r#"{"type":"hello","client_id":null,"token":null,"hostname":"h","name":"n"}"#;
        let AgentMsg::Hello { enroll_key, .. } = serde_json::from_str(old).unwrap() else { panic!() };
        assert_eq!(enroll_key, None);
        let hello = |enroll_key| AgentMsg::Hello {
            client_id: None,
            token: None,
            hostname: "h".into(),
            name: "n".into(),
            enroll_key,
        };
        assert!(!serde_json::to_string(&hello(None)).unwrap().contains("enroll_key"));
        assert!(serde_json::to_string(&hello(Some("k".into()))).unwrap().contains(r#""enroll_key":"k""#));
    }
}
