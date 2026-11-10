//! In-memory coordinator: owns the store, timer runs and live connections, and
//! fans state changes out to the control deck and to agents.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Local, Utc};
use firefight_core::model::{
    Action, ActionResult, ClientId, ClientStatus, ClientView, Repeat, RunId, TimerDef, TimerId, TimerKind,
};
use firefight_core::protocol::{AdminEvent, ServerMsg};
use firefight_core::run::{RunState, TimerRun};
use firefight_core::schedule::next_occurrence;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::store::{ClientRecord, Store, hash_token, new_token, token_matches};

pub type AgentTx = mpsc::UnboundedSender<ServerMsg>;

struct Conn {
    conn_id: Uuid,
    tx: AgentTx,
}

pub struct Hub {
    store: Store,
    runs: HashMap<TimerId, TimerRun>,
    agents: HashMap<ClientId, Conn>,
    last_results: HashMap<ClientId, ActionResult>,
    /// Clients that finished all actions of a timer's current run.
    completed: HashMap<TimerId, (RunId, HashSet<ClientId>)>,
    admin_tx: broadcast::Sender<AdminEvent>,
}

/// Result of an agent's `Hello`.
pub enum HelloOutcome {
    Accepted { client_id: ClientId, conn_id: Uuid, rx: mpsc::UnboundedReceiver<ServerMsg> },
    Rejected(String),
}

/// Duration a timer resets to when idle.
fn base_total_ms(def: &TimerDef) -> u64 {
    match def.kind {
        TimerKind::Countdown { duration_secs } => duration_secs * 1000,
        TimerKind::AtTime { .. } => 0,
    }
}

fn next_at(def: &TimerDef, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match &def.kind {
        TimerKind::AtTime { time, repeat } => {
            next_occurrence(*time, repeat, &now.with_timezone(&Local)).map(|t| t.with_timezone(&Utc))
        }
        TimerKind::Countdown { .. } => None,
    }
}

impl Hub {
    pub fn new(store: Store, now: DateTime<Utc>) -> Self {
        let (admin_tx, _) = broadcast::channel(256);
        let mut hub = Self {
            store,
            runs: HashMap::new(),
            agents: HashMap::new(),
            last_results: HashMap::new(),
            completed: HashMap::new(),
            admin_tx,
        };
        let defs = hub.store.data.timers.clone();
        for def in &defs {
            hub.runs.insert(def.id, hub.initial_run(def, now));
        }
        hub
    }

    /// Countdowns start idle; `AtTime` timers are armed for their next occurrence.
    fn initial_run(&self, def: &TimerDef, now: DateTime<Utc>) -> TimerRun {
        let mut run = TimerRun::idle(def.id, base_total_ms(def));
        if let Some(at) = next_at(def, now) {
            run.arm(at, now);
        }
        run
    }

    pub fn subscribe_admin(&self) -> broadcast::Receiver<AdminEvent> {
        self.admin_tx.subscribe()
    }

    fn emit(&self, ev: AdminEvent) {
        let _ = self.admin_tx.send(ev);
    }

    pub fn snapshot(&self, now: DateTime<Utc>) -> AdminEvent {
        AdminEvent::Snapshot {
            timers: self.store.data.timers.clone(),
            runs: self.store.data.timers.iter().filter_map(|t| self.runs.get(&t.id).cloned()).collect(),
            clients: self.store.data.clients.iter().map(|c| self.client_view(c)).collect(),
            server_now: now,
        }
    }

    fn client_view(&self, c: &ClientRecord) -> ClientView {
        ClientView {
            id: c.id,
            name: c.name.clone(),
            hostname: c.hostname.clone(),
            status: c.status,
            online: self.agents.contains_key(&c.id),
            last_seen: c.last_seen,
            assigned_timers: c.assigned_timers.clone(),
            last_result: self.last_results.get(&c.id).cloned(),
        }
    }

    fn emit_client(&self, id: ClientId) {
        if let Some(c) = self.store.client(id) {
            self.emit(AdminEvent::ClientUpdated { client: self.client_view(c) });
        }
    }

    fn save(&self) {
        if let Err(e) = self.store.save() {
            tracing::error!("failed to save state: {e:#}");
        }
    }

    fn send_agent(&self, id: ClientId, msg: ServerMsg) {
        if let Some(conn) = self.agents.get(&id) {
            let _ = conn.tx.send(msg);
        }
    }

    fn approved_clients_with(&self, timer_id: TimerId) -> impl Iterator<Item = &ClientRecord> {
        self.store
            .data
            .clients
            .iter()
            .filter(move |c| c.status == ClientStatus::Approved && c.assigned_timers.contains(&timer_id))
    }

    /// Send a client the full set of its assigned timers and their runs.
    fn push_assigned(&self, id: ClientId, now: DateTime<Utc>) {
        let Some(c) = self.store.client(id) else { return };
        if c.status != ClientStatus::Approved {
            return;
        }
        let timers: Vec<_> = c.assigned_timers.iter().filter_map(|t| self.store.timer(*t).cloned()).collect();
        let runs = timers.iter().filter_map(|t| self.runs.get(&t.id).cloned()).collect();
        self.send_agent(id, ServerMsg::Assigned { timers, runs, server_now: now });
    }

    fn push_assigned_for_timer(&self, timer_id: TimerId, now: DateTime<Utc>) {
        let ids: Vec<_> = self.approved_clients_with(timer_id).map(|c| c.id).collect();
        for id in ids {
            self.push_assigned(id, now);
        }
    }

    fn broadcast_run(&self, timer_id: TimerId, now: DateTime<Utc>) {
        let Some(run) = self.runs.get(&timer_id) else { return };
        self.emit(AdminEvent::RunUpdate { run: run.clone(), server_now: now });
        for c in self.approved_clients_with(timer_id) {
            self.send_agent(c.id, ServerMsg::RunUpdate { run: run.clone(), server_now: now });
        }
    }

    // ---- timers -------------------------------------------------------------

    pub fn timers(&self) -> &[TimerDef] {
        &self.store.data.timers
    }

    /// Create or replace a timer definition. Its run is reset.
    pub fn upsert_timer(&mut self, def: TimerDef, now: DateTime<Utc>) -> TimerDef {
        match self.store.data.timers.iter_mut().find(|t| t.id == def.id) {
            Some(existing) => *existing = def.clone(),
            None => self.store.data.timers.push(def.clone()),
        }
        self.save();
        let run = self.initial_run(&def, now);
        self.runs.insert(def.id, run.clone());
        self.emit(AdminEvent::TimerUpdated { timer: def.clone(), run });
        self.push_assigned_for_timer(def.id, now);
        def
    }

    pub fn delete_timer(&mut self, id: TimerId, now: DateTime<Utc>) -> Result<()> {
        let before = self.store.data.timers.len();
        self.store.data.timers.retain(|t| t.id != id);
        if self.store.data.timers.len() == before {
            bail!("no such timer");
        }
        let affected: Vec<_> = self.approved_clients_with(id).map(|c| c.id).collect();
        for c in &mut self.store.data.clients {
            c.assigned_timers.retain(|t| *t != id);
        }
        self.save();
        self.runs.remove(&id);
        self.emit(AdminEvent::TimerDeleted { id });
        for c in affected {
            self.push_assigned(c, now);
            self.emit_client(c);
        }
        Ok(())
    }

    /// Apply a control operation to a timer's run and broadcast the result.
    pub fn control(&mut self, id: TimerId, op: Control, now: DateTime<Utc>) -> Result<TimerRun> {
        let def = self.store.timer(id).cloned().ok_or_else(|| anyhow!("no such timer"))?;
        let total = base_total_ms(&def);
        let run = self.runs.get_mut(&id).ok_or_else(|| anyhow!("no such timer"))?;
        let at_time = matches!(def.kind, TimerKind::AtTime { .. });
        match op {
            Control::Start if at_time => {
                if run.state != RunState::Idle {
                    bail!("timer is already {:?}", run.state);
                }
                let at = next_at(&def, now).ok_or_else(|| anyhow!("timer has no upcoming occurrence"))?;
                run.arm(at, now);
            }
            Control::Start => run.start(now)?,
            Control::Pause if at_time => bail!("scheduled timers cannot be paused"),
            Control::Pause => run.pause(now)?,
            Control::Add { secs } => run.add(secs * 1000, now),
            Control::Reset => run.reset(total),
            Control::Dismiss => {
                run.dismiss(total)?;
                let run_id = run.run_id;
                // Repeating scheduled timers re-arm for their next occurrence.
                if let TimerKind::AtTime { repeat, .. } = &def.kind
                    && *repeat != Repeat::Once
                    && let Some(at) = next_at(&def, now)
                {
                    run.arm(at, now);
                }
                for c in self.approved_clients_with(id) {
                    self.send_agent(c.id, ServerMsg::Dismissed { timer_id: id, run_id });
                }
            }
        }
        self.broadcast_run(id, now);
        Ok(self.runs[&id].clone())
    }

    /// Move every due run to `Firing`. Called periodically by the engine.
    pub fn tick(&mut self, now: DateTime<Utc>) {
        let due: Vec<_> = self.runs.values().filter(|r| r.is_due(now)).map(|r| r.timer_id).collect();
        for id in due {
            if let Some(run) = self.runs.get_mut(&id) {
                let _ = run.fire();
                tracing::info!(timer = %id, run = %run.run_id, "timer fired");
            }
            self.broadcast_run(id, now);
        }
        // Also catches clients that disconnected or were unassigned mid-run.
        self.auto_complete_all(now);
    }

    /// Timers clients cannot dismiss return to idle on their own once every
    /// online assigned client has finished the run's actions.
    fn auto_complete(&mut self, timer_id: TimerId, now: DateTime<Utc>) {
        if self.store.timer(timer_id).is_none_or(|t| t.allow_client_dismiss) {
            return;
        }
        let Some(run) = self.runs.get(&timer_id).filter(|r| r.state == RunState::Firing) else { return };
        let done = self.completed.get(&timer_id).filter(|(run_id, _)| *run_id == run.run_id).map(|(_, set)| set);
        let finished = self
            .approved_clients_with(timer_id)
            .filter(|c| self.agents.contains_key(&c.id))
            .all(|c| done.is_some_and(|set| set.contains(&c.id)));
        if finished {
            tracing::info!(timer = %timer_id, run = %run.run_id, "all clients finished; returning to idle");
            self.completed.remove(&timer_id);
            if let Err(e) = self.control(timer_id, Control::Dismiss, now) {
                tracing::warn!("auto dismiss failed: {e}");
            }
        }
    }

    fn auto_complete_all(&mut self, now: DateTime<Utc>) {
        let ids: Vec<_> = self.runs.values().filter(|r| r.state == RunState::Firing).map(|r| r.timer_id).collect();
        for id in ids {
            self.auto_complete(id, now);
        }
    }

    // ---- clients ------------------------------------------------------------

    pub fn clients(&self) -> Vec<ClientView> {
        self.store.data.clients.iter().map(|c| self.client_view(c)).collect()
    }

    pub fn approve_client(&mut self, id: ClientId, now: DateTime<Utc>) -> Result<ClientView> {
        let c = self.store.client_mut(id).ok_or_else(|| anyhow!("no such client"))?;
        if c.status == ClientStatus::Approved {
            bail!("client is already approved");
        }
        c.status = ClientStatus::Approved;
        self.save();
        if self.agents.contains_key(&id) {
            self.issue_token(id);
            self.push_assigned(id, now);
        }
        self.emit_client(id);
        Ok(self.client_view(self.store.client(id).expect("exists")))
    }

    /// Generate and send a fresh token to a connected, approved client.
    fn issue_token(&mut self, id: ClientId) {
        let token = new_token();
        if let Some(c) = self.store.client_mut(id) {
            c.token_hash = Some(hash_token(&token));
        }
        self.save();
        self.send_agent(id, ServerMsg::Welcome { client_id: id, status: ClientStatus::Approved, token: Some(token) });
    }

    /// Reject a pending client or remove an approved one. A connected agent is told and dropped.
    pub fn remove_client(&mut self, id: ClientId, reason: &str) -> Result<()> {
        let before = self.store.data.clients.len();
        self.store.data.clients.retain(|c| c.id != id);
        if self.store.data.clients.len() == before {
            bail!("no such client");
        }
        self.save();
        if let Some(conn) = self.agents.remove(&id) {
            let _ = conn.tx.send(ServerMsg::Rejected { reason: reason.into() });
        }
        self.last_results.remove(&id);
        self.emit(AdminEvent::ClientDeleted { id });
        Ok(())
    }

    pub fn rename_client(&mut self, id: ClientId, name: String) -> Result<ClientView> {
        self.store.client_mut(id).ok_or_else(|| anyhow!("no such client"))?.name = name;
        self.save();
        self.emit_client(id);
        Ok(self.client_view(self.store.client(id).expect("exists")))
    }

    pub fn set_assignments(
        &mut self,
        id: ClientId,
        mut timers: Vec<TimerId>,
        now: DateTime<Utc>,
    ) -> Result<ClientView> {
        if let Some(bad) = timers.iter().find(|t| self.store.timer(**t).is_none()) {
            bail!("no such timer {bad}");
        }
        timers.dedup();
        self.store.client_mut(id).ok_or_else(|| anyhow!("no such client"))?.assigned_timers = timers;
        self.save();
        self.push_assigned(id, now);
        self.emit_client(id);
        Ok(self.client_view(self.store.client(id).expect("exists")))
    }

    pub fn test_run(&self, id: ClientId, actions: Vec<Action>) -> Result<()> {
        let c = self.store.client(id).ok_or_else(|| anyhow!("no such client"))?;
        if c.status != ClientStatus::Approved || !self.agents.contains_key(&id) {
            bail!("client is not online and approved");
        }
        self.send_agent(id, ServerMsg::TestRun { actions });
        Ok(())
    }

    // ---- agent connections --------------------------------------------------

    pub fn agent_hello(
        &mut self,
        client_id: Option<ClientId>,
        token: Option<String>,
        hostname: String,
        name: String,
        now: DateTime<Utc>,
    ) -> HelloOutcome {
        let id = match client_id {
            Some(id) => {
                let Some(c) = self.store.client(id) else {
                    return HelloOutcome::Rejected("unknown client".into());
                };
                let ok = match (&c.token_hash, &token) {
                    (Some(hash), Some(t)) => token_matches(t, hash),
                    // A token was issued but none presented: someone else may be using this id.
                    (Some(_), None) => false,
                    // Not yet issued; the agent must not claim to have one.
                    (None, t) => t.is_none(),
                };
                if !ok {
                    return HelloOutcome::Rejected("invalid credentials".into());
                }
                id
            }
            None => {
                let id = Uuid::new_v4();
                self.store.data.clients.push(ClientRecord {
                    id,
                    name: name.clone(),
                    hostname: hostname.clone(),
                    status: ClientStatus::Pending,
                    token_hash: None,
                    assigned_timers: vec![],
                    last_seen: None,
                });
                tracing::info!(client = %id, %hostname, "new client awaiting approval");
                id
            }
        };

        let c = self.store.client_mut(id).expect("exists");
        c.hostname = hostname;
        c.last_seen = Some(now);
        let status = c.status;
        let needs_token = status == ClientStatus::Approved && c.token_hash.is_none();
        self.save();

        let (tx, rx) = mpsc::unbounded_channel();
        let conn_id = Uuid::new_v4();
        // Dropping a previous connection's sender ends that session.
        self.agents.insert(id, Conn { conn_id, tx });
        if needs_token {
            self.issue_token(id);
        } else {
            self.send_agent(id, ServerMsg::Welcome { client_id: id, status, token: None });
        }
        self.push_assigned(id, now);
        self.emit_client(id);
        HelloOutcome::Accepted { client_id: id, conn_id, rx }
    }

    pub fn agent_disconnected(&mut self, id: ClientId, conn_id: Uuid, now: DateTime<Utc>) {
        if self.agents.get(&id).is_some_and(|c| c.conn_id == conn_id) {
            self.agents.remove(&id);
            if let Some(c) = self.store.client_mut(id) {
                c.last_seen = Some(now);
                self.save();
            }
            self.emit_client(id);
        }
    }

    pub fn agent_result(&mut self, id: ClientId, result: ActionResult) {
        tracing::info!(client = %id, ok = result.ok, code = ?result.exit_code, "action result: {}", result.message);
        self.last_results.insert(id, result);
        self.emit_client(id);
    }

    /// Dismiss from a client display; only allowed for timers assigned to it
    /// that permit client dismissal.
    pub fn agent_dismiss(&mut self, id: ClientId, timer_id: TimerId, now: DateTime<Utc>) {
        let allowed = self
            .store
            .client(id)
            .is_some_and(|c| c.status == ClientStatus::Approved && c.assigned_timers.contains(&timer_id))
            && self.store.timer(timer_id).is_some_and(|t| t.allow_client_dismiss);
        if allowed && let Err(e) = self.control(timer_id, Control::Dismiss, now) {
            tracing::debug!("client dismiss ignored: {e}");
        }
    }

    /// An agent finished every action of a run. Recorded even if the server has
    /// not moved the run to `Firing` yet, since the agent may fire first.
    pub fn agent_run_complete(&mut self, id: ClientId, timer_id: TimerId, run_id: RunId, now: DateTime<Utc>) {
        if self.runs.get(&timer_id).is_none_or(|r| r.run_id != run_id) {
            return;
        }
        let entry = self.completed.entry(timer_id).or_insert_with(|| (run_id, HashSet::new()));
        if entry.0 != run_id {
            *entry = (run_id, HashSet::new());
        }
        entry.1.insert(id);
        self.auto_complete(timer_id, now);
    }

    pub fn is_approved(&self, id: ClientId) -> bool {
        self.store.client(id).is_some_and(|c| c.status == ClientStatus::Approved)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Control {
    Start,
    Pause,
    Add { secs: u64 },
    Reset,
    Dismiss,
}

#[cfg(test)]
mod tests {
    use super::*;
    use firefight_core::model::{ActionKind, ActionMode};

    fn hub() -> Hub {
        let dir = std::env::temp_dir().join(format!("firefight-test-{}", Uuid::new_v4()));
        Hub::new(Store::open(&dir).unwrap(), Utc::now())
    }

    fn hello(hub: &mut Hub, id: Option<ClientId>, token: Option<String>) -> Result<(ClientId, Vec<ServerMsg>), String> {
        match hub.agent_hello(id, token, "host".into(), "agent".into(), Utc::now()) {
            HelloOutcome::Accepted { client_id, mut rx, .. } => {
                let mut msgs = vec![];
                while let Ok(m) = rx.try_recv() {
                    msgs.push(m);
                }
                Ok((client_id, msgs))
            }
            HelloOutcome::Rejected(r) => Err(r),
        }
    }

    fn issued_token(msgs: &[ServerMsg]) -> Option<String> {
        msgs.iter().find_map(|m| match m {
            ServerMsg::Welcome { token, .. } => token.clone(),
            _ => None,
        })
    }

    fn countdown(secs: u64) -> TimerDef {
        TimerDef {
            id: Uuid::new_v4(),
            name: "t".into(),
            kind: TimerKind::Countdown { duration_secs: secs },
            actions: vec![Action {
                id: Uuid::new_v4(),
                kind: ActionKind::Command { cmd: "true".into() },
                mode: ActionMode::Parallel,
            }],
            allow_client_dismiss: true,
        }
    }

    #[test]
    fn pairing_issues_token_on_reconnect_after_offline_approval() {
        let mut hub = hub();
        let (id, msgs) = hello(&mut hub, None, None).unwrap();
        assert!(matches!(msgs[0], ServerMsg::Welcome { status: ClientStatus::Pending, token: None, .. }));
        hub.agent_disconnected(id, hub.agents[&id].conn_id, Utc::now());

        hub.approve_client(id, Utc::now()).unwrap();
        let (_, msgs) = hello(&mut hub, Some(id), None).unwrap();
        let token = issued_token(&msgs).expect("token issued on first approved connect");

        // Token is now required.
        assert!(hello(&mut hub, Some(id), None).is_err());
        assert!(hello(&mut hub, Some(id), Some("wrong".into())).is_err());
        let (_, msgs) = hello(&mut hub, Some(id), Some(token)).unwrap();
        assert!(issued_token(&msgs).is_none());
    }

    #[test]
    fn unknown_or_removed_clients_are_rejected() {
        let mut hub = hub();
        assert!(hello(&mut hub, Some(Uuid::new_v4()), None).is_err());
        let (id, _) = hello(&mut hub, None, None).unwrap();
        hub.remove_client(id, "no").unwrap();
        assert!(hello(&mut hub, Some(id), None).is_err());
    }

    #[test]
    fn only_assigned_approved_clients_get_runs() {
        let mut hub = hub();
        let now = Utc::now();
        let def = hub.upsert_timer(countdown(60), now);
        let (a, _) = hello(&mut hub, None, None).unwrap();
        let (b, _) = hello(&mut hub, None, None).unwrap();
        hub.approve_client(a, now).unwrap();
        hub.approve_client(b, now).unwrap();
        hub.set_assignments(a, vec![def.id], now).unwrap();

        // Swap in receivers we can inspect.
        let (tx_a, mut rx_a) = mpsc::unbounded_channel();
        let (tx_b, mut rx_b) = mpsc::unbounded_channel();
        hub.agents.insert(a, Conn { conn_id: Uuid::new_v4(), tx: tx_a });
        hub.agents.insert(b, Conn { conn_id: Uuid::new_v4(), tx: tx_b });

        hub.control(def.id, Control::Start, now).unwrap();
        assert!(matches!(rx_a.try_recv(), Ok(ServerMsg::RunUpdate { .. })));
        assert!(rx_b.try_recv().is_err());

        // Client b cannot dismiss a timer it is not assigned.
        hub.tick(now + chrono::Duration::seconds(61));
        hub.agent_dismiss(b, def.id, now);
        assert_eq!(hub.runs[&def.id].state, RunState::Firing);
        hub.agent_dismiss(a, def.id, now);
        assert_eq!(hub.runs[&def.id].state, RunState::Idle);
    }

    #[test]
    fn client_dismiss_respects_timer_setting() {
        let mut hub = hub();
        let now = Utc::now();
        let def = hub.upsert_timer(TimerDef { allow_client_dismiss: false, ..countdown(60) }, now);
        let (a, _) = hello(&mut hub, None, None).unwrap();
        hub.approve_client(a, now).unwrap();
        hub.set_assignments(a, vec![def.id], now).unwrap();

        hub.control(def.id, Control::Start, now).unwrap();
        hub.tick(now + chrono::Duration::seconds(61));
        hub.agent_dismiss(a, def.id, now);
        assert_eq!(hub.runs[&def.id].state, RunState::Firing);
        // The control deck can still dismiss it.
        hub.control(def.id, Control::Dismiss, now).unwrap();
        assert_eq!(hub.runs[&def.id].state, RunState::Idle);
    }

    #[test]
    fn non_dismissable_timer_idles_after_all_clients_complete() {
        let mut hub = hub();
        let now = Utc::now();
        let def = hub.upsert_timer(TimerDef { allow_client_dismiss: false, ..countdown(60) }, now);
        let (a, _) = hello(&mut hub, None, None).unwrap();
        let (b, _) = hello(&mut hub, None, None).unwrap();
        for c in [a, b] {
            hub.approve_client(c, now).unwrap();
            hub.set_assignments(c, vec![def.id], now).unwrap();
        }
        hub.control(def.id, Control::Start, now).unwrap();
        let run_id = hub.runs[&def.id].run_id;

        // Agent a finishes before the server's own tick marks the run as firing.
        hub.agent_run_complete(a, def.id, run_id, now);
        hub.tick(now + chrono::Duration::seconds(61));
        assert_eq!(hub.runs[&def.id].state, RunState::Firing, "still waiting on b");

        // A stale completion for another run is ignored.
        hub.agent_run_complete(b, def.id, Uuid::new_v4(), now);
        assert_eq!(hub.runs[&def.id].state, RunState::Firing);

        hub.agent_run_complete(b, def.id, run_id, now);
        assert_eq!(hub.runs[&def.id].state, RunState::Idle);
    }

    #[test]
    fn dismissable_timer_keeps_firing_after_completion() {
        let mut hub = hub();
        let now = Utc::now();
        let def = hub.upsert_timer(countdown(60), now);
        let (a, _) = hello(&mut hub, None, None).unwrap();
        hub.approve_client(a, now).unwrap();
        hub.set_assignments(a, vec![def.id], now).unwrap();
        hub.control(def.id, Control::Start, now).unwrap();
        hub.tick(now + chrono::Duration::seconds(61));
        hub.agent_run_complete(a, def.id, hub.runs[&def.id].run_id, now);
        assert_eq!(hub.runs[&def.id].state, RunState::Firing);
    }

    #[test]
    fn state_persists_across_restart() {
        let dir = std::env::temp_dir().join(format!("firefight-test-{}", Uuid::new_v4()));
        let def = {
            let mut hub = Hub::new(Store::open(&dir).unwrap(), Utc::now());
            hub.upsert_timer(countdown(30), Utc::now())
        };
        let hub = Hub::new(Store::open(&dir).unwrap(), Utc::now());
        assert_eq!(hub.timers(), &[def]);
    }
}
