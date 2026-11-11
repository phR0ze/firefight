//! In-memory coordinator: owns the store, timer runs and live connections, and
//! fans state changes out to the control deck and to agents.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Local, Utc};
use firefight_core::model::{
    Action, ActionResult, ClientId, ClientStatus, ClientView, Repeat, RunId, TimerDef, TimerId, TimerKind,
};
use firefight_core::protocol::{AdminEvent, ClientRun, ServerMsg};
use firefight_core::run::{RunState, TimerRun};
use firefight_core::schedule::next_occurrence;
use tokio::sync::{broadcast, mpsc};
use uuid::Uuid;

use crate::config::Resolved;
use crate::store::{ClientRecord, Store, hash_token, new_token, token_matches, token_matches_plain};

pub type AgentTx = mpsc::UnboundedSender<ServerMsg>;

struct Conn {
    conn_id: Uuid,
    tx: AgentTx,
}

pub struct Hub {
    store: Store,
    /// Shared run of each timer, followed by every assigned client without its own copy.
    runs: HashMap<TimerId, TimerRun>,
    /// Copies of a timer's run that a client follows on its own, made when the timers of one
    /// client are controlled from its card. Resetting from the deck drops them.
    detached: HashMap<(TimerId, ClientId), TimerRun>,
    agents: HashMap<ClientId, Conn>,
    last_results: HashMap<ClientId, ActionResult>,
    /// Clients that finished all actions of a run, keyed by timer and the client whose copy it
    /// is (`None` for the shared run).
    completed: HashMap<(TimerId, Option<ClientId>), (RunId, HashSet<ClientId>)>,
    /// Shared key that auto-approves agents presenting it.
    enroll_key: Option<String>,
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

/// Apply a control operation to one run of `def`.
fn apply(def: &TimerDef, run: &mut TimerRun, op: Control, now: DateTime<Utc>) -> Result<()> {
    let total = base_total_ms(def);
    let at_time = matches!(def.kind, TimerKind::AtTime { .. });
    match op {
        Control::Start if at_time => {
            if run.state != RunState::Idle {
                bail!("timer is already {:?}", run.state);
            }
            let at = next_at(def, now).ok_or_else(|| anyhow!("timer has no upcoming occurrence"))?;
            run.arm(at, now);
        }
        Control::Start => run.start(now)?,
        Control::Pause if at_time => bail!("scheduled timers cannot be paused"),
        Control::Pause => run.pause(now)?,
        Control::Add { secs } => run.add(secs * 1000, now),
        Control::Reset => run.reset(total),
        Control::Dismiss => {
            run.dismiss(total)?;
            // Repeating scheduled timers re-arm for their next occurrence.
            if let TimerKind::AtTime { repeat, .. } = &def.kind
                && *repeat != Repeat::Once
                && let Some(at) = next_at(def, now)
            {
                run.arm(at, now);
            }
        }
    }
    Ok(())
}

impl Hub {
    pub fn new(store: Store, now: DateTime<Utc>) -> Self {
        let (admin_tx, _) = broadcast::channel(256);
        let mut hub = Self {
            store,
            runs: HashMap::new(),
            detached: HashMap::new(),
            agents: HashMap::new(),
            last_results: HashMap::new(),
            completed: HashMap::new(),
            enroll_key: None,
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
            client_runs: self
                .detached
                .iter()
                .map(|((_, client_id), run)| ClientRun { client_id: *client_id, run: run.clone() })
                .collect(),
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

    /// Approved clients with the timer that follow its shared run.
    fn shared_clients_with(&self, timer_id: TimerId) -> impl Iterator<Item = &ClientRecord> {
        self.approved_clients_with(timer_id).filter(move |c| !self.detached.contains_key(&(timer_id, c.id)))
    }

    /// The run a client follows for a timer: its own copy, else the shared run.
    fn run_for(&self, timer_id: TimerId, client: ClientId) -> Option<&TimerRun> {
        self.detached.get(&(timer_id, client)).or_else(|| self.runs.get(&timer_id))
    }

    /// Clients following their own copy of a timer's run.
    fn copies_of(&self, timer_id: TimerId) -> Vec<ClientId> {
        self.detached.keys().filter(|(t, _)| *t == timer_id).map(|(_, c)| *c).collect()
    }

    /// Send a client the full set of its assigned timers and their runs.
    fn push_assigned(&self, id: ClientId, now: DateTime<Utc>) {
        let Some(c) = self.store.client(id) else { return };
        if c.status != ClientStatus::Approved {
            return;
        }
        let timers: Vec<_> = c.assigned_timers.iter().filter_map(|t| self.store.timer(*t).cloned()).collect();
        let runs = timers.iter().filter_map(|t| self.run_for(t.id, id).cloned()).collect();
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
        for c in self.shared_clients_with(timer_id) {
            self.send_agent(c.id, ServerMsg::RunUpdate { run: run.clone(), server_now: now });
        }
    }

    fn broadcast_detached(&self, timer_id: TimerId, client_id: ClientId, now: DateTime<Utc>) {
        let Some(run) = self.detached.get(&(timer_id, client_id)) else { return };
        self.emit(AdminEvent::ClientRunUpdate { client_id, run: run.clone(), server_now: now });
        self.send_agent(client_id, ServerMsg::RunUpdate { run: run.clone(), server_now: now });
    }

    /// Put a client back on a timer's shared run. Returns whether it had its own copy.
    fn drop_detached(&mut self, timer_id: TimerId, client_id: ClientId, now: DateTime<Utc>) -> bool {
        if self.detached.remove(&(timer_id, client_id)).is_none() {
            return false;
        }
        self.emit(AdminEvent::ClientRunRemoved { client_id, timer_id });
        if let Some(run) = self.runs.get(&timer_id) {
            self.send_agent(client_id, ServerMsg::RunUpdate { run: run.clone(), server_now: now });
        }
        true
    }

    /// A client's copy that is idle while the shared run is idle too is no longer needed.
    fn maybe_rejoin(&mut self, timer_id: TimerId, client_id: ClientId, now: DateTime<Utc>) {
        let idle = |r: Option<&TimerRun>| r.is_some_and(|r| r.state == RunState::Idle);
        if idle(self.detached.get(&(timer_id, client_id))) && idle(self.runs.get(&timer_id)) {
            self.drop_detached(timer_id, client_id, now);
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
        for c in self.copies_of(def.id) {
            self.drop_detached(def.id, c, now);
        }
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
        for c in self.copies_of(id) {
            self.drop_detached(id, c, now);
        }
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

    /// Apply a control operation from the deck to every client with the timer. Resetting puts
    /// clients following their own copy back on the shared run; starting, pausing and dismissing
    /// apply to those copies in place.
    pub fn control(&mut self, id: TimerId, op: Control, now: DateTime<Utc>) -> Result<TimerRun> {
        let def = self.store.timer(id).cloned().ok_or_else(|| anyhow!("no such timer"))?;
        let copies = self.copies_of(id);
        let shared = self.control_shared(id, op, now);
        let mut copies_done = false;
        for c in copies {
            copies_done |= match op {
                Control::Reset => self.drop_detached(id, c, now),
                Control::Start | Control::Pause | Control::Dismiss => self.control_detached(&def, c, op, now).is_ok(),
                Control::Add { .. } => false,
            };
        }
        if let Err(e) = shared
            && !copies_done
        {
            return Err(e);
        }
        Ok(self.runs[&id].clone())
    }

    /// Apply a control operation to a timer's shared run and broadcast the result.
    fn control_shared(&mut self, id: TimerId, op: Control, now: DateTime<Utc>) -> Result<()> {
        let def = self.store.timer(id).cloned().ok_or_else(|| anyhow!("no such timer"))?;
        let run = self.runs.get_mut(&id).ok_or_else(|| anyhow!("no such timer"))?;
        let run_id = run.run_id;
        apply(&def, run, op, now)?;
        if let Control::Dismiss = op {
            for c in self.shared_clients_with(id) {
                self.send_agent(c.id, ServerMsg::Dismissed { timer_id: id, run_id });
            }
        }
        self.broadcast_run(id, now);
        // Clients waiting on an idle copy rejoin once the shared run is idle too.
        for c in self.copies_of(id) {
            self.maybe_rejoin(id, c, now);
        }
        Ok(())
    }

    /// Apply a control operation to one client's copy of a timer's run.
    fn control_detached(&mut self, def: &TimerDef, client: ClientId, op: Control, now: DateTime<Utc>) -> Result<()> {
        let run = self.detached.get_mut(&(def.id, client)).ok_or_else(|| anyhow!("no such run"))?;
        let run_id = run.run_id;
        apply(def, run, op, now)?;
        if let Control::Dismiss = op {
            self.send_agent(client, ServerMsg::Dismissed { timer_id: def.id, run_id });
        }
        self.broadcast_detached(def.id, client, now);
        self.maybe_rejoin(def.id, client, now);
        Ok(())
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
        let due: Vec<_> = self.detached.iter().filter(|(_, r)| r.is_due(now)).map(|(k, _)| *k).collect();
        for (id, client) in due {
            if let Some(run) = self.detached.get_mut(&(id, client)) {
                let _ = run.fire();
                tracing::info!(timer = %id, %client, run = %run.run_id, "client's timer fired");
            }
            self.broadcast_detached(id, client, now);
        }
        // Also catches clients that disconnected or were unassigned mid-run.
        self.auto_complete_all(now);
    }

    /// Timers clients cannot dismiss return to idle on their own once every
    /// online client following the run has finished its actions. `client` picks
    /// that client's copy of the run instead of the shared one.
    fn auto_complete(&mut self, timer_id: TimerId, client: Option<ClientId>, now: DateTime<Utc>) {
        let Some(def) = self.store.timer(timer_id).filter(|t| !t.allow_client_dismiss).cloned() else { return };
        let run = match client {
            Some(c) => self.detached.get(&(timer_id, c)),
            None => self.runs.get(&timer_id),
        };
        let Some(run) = run.filter(|r| r.state == RunState::Firing) else { return };
        let done =
            self.completed.get(&(timer_id, client)).filter(|(run_id, _)| *run_id == run.run_id).map(|(_, set)| set);
        let is_done = |c: ClientId| !self.agents.contains_key(&c) || done.is_some_and(|set| set.contains(&c));
        let finished = match client {
            Some(c) => is_done(c),
            None => self.shared_clients_with(timer_id).all(|c| is_done(c.id)),
        };
        if finished {
            tracing::info!(timer = %timer_id, run = %run.run_id, "all clients finished; returning to idle");
            self.completed.remove(&(timer_id, client));
            let res = match client {
                Some(c) => self.control_detached(&def, c, Control::Dismiss, now),
                None => self.control_shared(timer_id, Control::Dismiss, now),
            };
            if let Err(e) = res {
                tracing::warn!("auto dismiss failed: {e}");
            }
        }
    }

    fn auto_complete_all(&mut self, now: DateTime<Utc>) {
        let firing = |r: &TimerRun| r.state == RunState::Firing;
        let mut runs: Vec<_> = self.runs.values().filter(|r| firing(r)).map(|r| (r.timer_id, None)).collect();
        runs.extend(self.detached.iter().filter(|(_, r)| firing(r)).map(|((t, c), _)| (*t, Some(*c))));
        for (id, client) in runs {
            self.auto_complete(id, client, now);
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
        self.detached.retain(|(_, c), _| *c != id);
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
        let client = self.store.client_mut(id).ok_or_else(|| anyhow!("no such client"))?;
        let unassigned: Vec<_> = client.assigned_timers.iter().filter(|t| !timers.contains(t)).copied().collect();
        client.assigned_timers = timers;
        self.save();
        for t in unassigned {
            self.drop_detached(t, id, now);
        }
        self.push_assigned(id, now);
        self.emit_client(id);
        Ok(self.client_view(self.store.client(id).expect("exists")))
    }

    /// Apply `op` to every timer assigned to a client, all at the same instant,
    /// for that client only: it follows its own copy of each run it changes,
    /// leaving other clients with the same timer untouched. Timers the
    /// operation does not apply to are skipped, and one failing does not block
    /// the rest.
    pub fn bulk_client(&mut self, id: ClientId, op: Bulk, now: DateTime<Utc>) -> Result<Vec<TimerRun>> {
        let c = self.store.client(id).ok_or_else(|| anyhow!("no such client"))?;
        let ids = c.assigned_timers.clone();
        let mut done = vec![];
        for t in ids {
            let (Some(def), Some(run)) = (self.store.timer(t).cloned(), self.run_for(t, id).cloned()) else {
                continue;
            };
            if !op.applies(&def, &run) {
                continue;
            }
            match self.control_own(&def, id, op.control(), now) {
                Ok(run) => done.push(run),
                Err(e) => tracing::warn!(timer = %t, client = %id, "{op:?} failed: {e}"),
            }
        }
        if done.is_empty() {
            bail!("{}", op.nothing_to_do());
        }
        Ok(done)
    }

    /// Apply a control operation from a client's card to one of its timers, for that client
    /// only (see [`Hub::bulk_client`]).
    pub fn control_client(
        &mut self,
        id: ClientId,
        timer_id: TimerId,
        op: Control,
        now: DateTime<Utc>,
    ) -> Result<TimerRun> {
        let c = self.store.client(id).ok_or_else(|| anyhow!("no such client"))?;
        if c.status != ClientStatus::Approved {
            bail!("client is not approved");
        }
        if !c.assigned_timers.contains(&timer_id) {
            bail!("timer is not assigned to this client");
        }
        let def = self.store.timer(timer_id).cloned().ok_or_else(|| anyhow!("no such timer"))?;
        let firing = self.run_for(timer_id, id).is_some_and(|r| r.state == RunState::Firing);
        if let Control::Add { .. } = op
            && matches!(def.kind, TimerKind::AtTime { .. })
            && !firing
        {
            bail!("scheduled timers can only be snoozed while firing");
        }
        self.control_own(&def, id, op, now)
    }

    /// Apply `op` to the run a client follows for `def`, first giving the client its own copy of
    /// the shared run if it has none. Returns the run the client follows afterwards.
    fn control_own(&mut self, def: &TimerDef, id: ClientId, op: Control, now: DateTime<Utc>) -> Result<TimerRun> {
        let copied = !self.detached.contains_key(&(def.id, id));
        if copied {
            let run = self.runs.get(&def.id).ok_or_else(|| anyhow!("no such timer"))?;
            // A new run id keeps the copy's firing and completion apart from the shared run's.
            self.detached.insert((def.id, id), TimerRun { run_id: Uuid::new_v4(), ..run.clone() });
        }
        if let Err(e) = self.control_detached(def, id, op, now) {
            if copied {
                self.detached.remove(&(def.id, id));
            }
            return Err(e);
        }
        Ok(self.run_for(def.id, id).cloned().expect("timer exists"))
    }

    /// Apply `op` to every timer assigned to at least one approved client, all
    /// at the same instant, for every client with it (see [`Hub::control`]).
    /// Each timer is touched once, however many clients share it. Timers the
    /// operation does not apply to are skipped, and one timer failing (e.g. a
    /// scheduled timer with no upcoming occurrence) does not block the rest.
    pub fn bulk_all(&mut self, op: Bulk, now: DateTime<Utc>) -> Result<Vec<TimerRun>> {
        let mut seen = HashSet::new();
        let ids: Vec<_> = self
            .store
            .data
            .clients
            .iter()
            .filter(|c| c.status == ClientStatus::Approved)
            .flat_map(|c| c.assigned_timers.iter().copied())
            .filter(|t| seen.insert(*t))
            .collect();
        let ids: Vec<_> = ids
            .into_iter()
            .filter(|t| match (self.store.timer(*t), self.runs.get(t)) {
                (Some(def), Some(run)) => {
                    op.applies(def, run)
                        || self.detached.iter().any(|((dt, _), copy)| {
                            // Resetting also brings clients back onto the shared run.
                            dt == t && (op == Bulk::Reset || op.applies(def, copy))
                        })
                }
                _ => false,
            })
            .collect();
        let done: Vec<_> = ids
            .into_iter()
            .filter_map(|t| {
                self.control(t, op.control(), now)
                    .inspect_err(|e| tracing::warn!(timer = %t, "{op:?} failed: {e}"))
                    .ok()
            })
            .collect();
        if done.is_empty() {
            bail!("{}", op.nothing_to_do());
        }
        Ok(done)
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
        enroll_key: Option<String>,
        now: DateTime<Utc>,
    ) -> HelloOutcome {
        let enrolled = match (&self.enroll_key, &enroll_key) {
            (Some(expected), Some(presented)) => token_matches_plain(presented, expected),
            _ => false,
        };
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
                if enrolled && c.status == ClientStatus::Pending {
                    tracing::info!(client = %id, "pending client approved by enrollment key");
                    self.store.client_mut(id).expect("exists").status = ClientStatus::Approved;
                }
                id
            }
            None => {
                let id = Uuid::new_v4();
                self.store.data.clients.push(ClientRecord {
                    id,
                    name: name.clone(),
                    hostname: hostname.clone(),
                    status: if enrolled { ClientStatus::Approved } else { ClientStatus::Pending },
                    token_hash: None,
                    assigned_timers: vec![],
                    last_seen: None,
                });
                if enrolled {
                    tracing::info!(client = %id, %hostname, "new client approved by enrollment key");
                } else {
                    tracing::info!(client = %id, %hostname, "new client awaiting approval");
                }
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

    // ---- declarative config -------------------------------------------------

    pub fn set_enroll_key(&mut self, key: Option<String>) {
        self.enroll_key = key;
    }

    /// Apply the declarative config: declared timers are created or overwritten, and timers an
    /// earlier config declared but this one doesn't are deleted. Which clients get a timer is left
    /// to the control deck; timers created in the deck are untouched.
    pub fn apply_config(&mut self, config: Resolved, now: DateTime<Utc>) {
        let declared: Vec<_> = config.timers.iter().map(|t| t.id).collect();
        for def in config.timers {
            self.upsert_timer(def, now);
        }
        let stale: Vec<_> =
            self.store.data.managed_timers.iter().filter(|id| !declared.contains(id)).copied().collect();
        for id in stale {
            tracing::info!(timer = %id, "removing timer no longer in the config");
            let _ = self.delete_timer(id, now);
        }
        self.store.data.managed_timers = declared;
        self.save();
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
    /// that permit client dismissal. Dismisses the run the client follows: its
    /// own copy, or the shared run of every client without one.
    pub fn agent_dismiss(&mut self, id: ClientId, timer_id: TimerId, now: DateTime<Utc>) {
        let allowed = self
            .store
            .client(id)
            .is_some_and(|c| c.status == ClientStatus::Approved && c.assigned_timers.contains(&timer_id));
        let Some(def) = self.store.timer(timer_id).filter(|t| allowed && t.allow_client_dismiss).cloned() else {
            return;
        };
        let res = if self.detached.contains_key(&(timer_id, id)) {
            self.control_detached(&def, id, Control::Dismiss, now)
        } else {
            self.control_shared(timer_id, Control::Dismiss, now)
        };
        if let Err(e) = res {
            tracing::debug!("client dismiss ignored: {e}");
        }
    }

    /// An agent finished every action of a run. Recorded even if the server has
    /// not moved the run to `Firing` yet, since the agent may fire first.
    pub fn agent_run_complete(&mut self, id: ClientId, timer_id: TimerId, run_id: RunId, now: DateTime<Utc>) {
        if self.run_for(timer_id, id).is_none_or(|r| r.run_id != run_id) {
            return;
        }
        let copy = self.detached.contains_key(&(timer_id, id)).then_some(id);
        let entry = self.completed.entry((timer_id, copy)).or_insert_with(|| (run_id, HashSet::new()));
        if entry.0 != run_id {
            *entry = (run_id, HashSet::new());
        }
        entry.1.insert(id);
        self.auto_complete(timer_id, copy, now);
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

/// An operation applied to many timers at once from the control deck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bulk {
    /// Start idle timers and resume paused ones.
    Start,
    /// Pause running countdowns (scheduled timers cannot be paused).
    Pause,
    /// Return every timer not already at its starting state to idle.
    Reset,
}

impl Bulk {
    fn applies(self, def: &TimerDef, run: &TimerRun) -> bool {
        match self {
            Bulk::Start => matches!(run.state, RunState::Idle | RunState::Paused),
            Bulk::Pause => run.state == RunState::Running && matches!(def.kind, TimerKind::Countdown { .. }),
            Bulk::Reset => !(run.state == RunState::Idle && run.remaining_ms == base_total_ms(def)),
        }
    }

    fn control(self) -> Control {
        match self {
            Bulk::Start => Control::Start,
            Bulk::Pause => Control::Pause,
            Bulk::Reset => Control::Reset,
        }
    }

    fn nothing_to_do(self) -> &'static str {
        match self {
            Bulk::Start => "no idle or paused timers to start",
            Bulk::Pause => "no running countdowns to pause",
            Bulk::Reset => "no timers to reset",
        }
    }
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
        match hub.agent_hello(id, token, "host".into(), "agent".into(), None, Utc::now()) {
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
    fn start_client_timers_starts_idle_and_paused_only() {
        let mut hub = hub();
        let now = Utc::now();
        let idle = hub.upsert_timer(countdown(60), now);
        let paused = hub.upsert_timer(countdown(60), now);
        let running = hub.upsert_timer(countdown(60), now);
        let other = hub.upsert_timer(countdown(60), now);
        let (a, _) = hello(&mut hub, None, None).unwrap();
        hub.approve_client(a, now).unwrap();
        hub.set_assignments(a, vec![idle.id, paused.id, running.id], now).unwrap();
        hub.control(paused.id, Control::Start, now).unwrap();
        hub.control(paused.id, Control::Pause, now).unwrap();
        hub.control(running.id, Control::Start, now).unwrap();
        let running_end = hub.runs[&running.id].ends_at;

        let later = now + chrono::Duration::seconds(5);
        let started = hub.bulk_client(a, Bulk::Start, later).unwrap();
        assert_eq!(started.len(), 2);
        assert_eq!(hub.run_for(idle.id, a).unwrap().state, RunState::Running);
        assert_eq!(hub.run_for(paused.id, a).unwrap().state, RunState::Running);
        assert_eq!(hub.run_for(running.id, a).unwrap().ends_at, running_end, "running timer untouched");
        assert!(!hub.detached.contains_key(&(running.id, a)), "running timer stays shared");
        assert_eq!(hub.runs[&other.id].state, RunState::Idle, "unassigned timer untouched");

        assert!(hub.bulk_client(a, Bulk::Start, later).is_err(), "nothing left to start");
    }

    /// Two approved clients sharing one countdown, each with an inspectable receiver.
    fn two_clients_one_timer(
        hub: &mut Hub,
        def: TimerDef,
    ) -> (TimerDef, [(ClientId, mpsc::UnboundedReceiver<ServerMsg>); 2]) {
        let now = Utc::now();
        let def = hub.upsert_timer(def, now);
        let clients = [(), ()].map(|_| {
            let (c, _) = hello(hub, None, None).unwrap();
            hub.approve_client(c, now).unwrap();
            hub.set_assignments(c, vec![def.id], now).unwrap();
            let (tx, rx) = mpsc::unbounded_channel();
            hub.agents.insert(c, Conn { conn_id: Uuid::new_v4(), tx });
            (c, rx)
        });
        (def, clients)
    }

    fn drain(rx: &mut mpsc::UnboundedReceiver<ServerMsg>) -> Vec<ServerMsg> {
        std::iter::from_fn(|| rx.try_recv().ok()).collect()
    }

    #[test]
    fn client_start_only_affects_that_client() {
        let mut hub = hub();
        let now = Utc::now();
        let (def, [(a, mut rx_a), (b, mut rx_b)]) = two_clients_one_timer(&mut hub, countdown(60));

        hub.bulk_client(a, Bulk::Start, now).unwrap();
        assert_eq!(hub.run_for(def.id, a).unwrap().state, RunState::Running);
        assert_eq!(hub.run_for(def.id, b).unwrap().state, RunState::Idle);
        assert_eq!(hub.runs[&def.id].state, RunState::Idle, "shared run untouched");
        assert!(matches!(&drain(&mut rx_a)[..], [ServerMsg::RunUpdate { run, .. }] if run.state == RunState::Running));
        assert!(drain(&mut rx_b).is_empty(), "b is not told about a's run");

        // b starts later, separately.
        let later = now + chrono::Duration::seconds(10);
        hub.bulk_client(b, Bulk::Start, later).unwrap();
        let (ra, rb) = (hub.run_for(def.id, a).unwrap(), hub.run_for(def.id, b).unwrap());
        assert_ne!(ra.run_id, rb.run_id);
        assert_ne!(ra.ends_at, rb.ends_at);

        // a's copy fires on its own; b keeps counting.
        hub.tick(now + chrono::Duration::seconds(61));
        assert_eq!(hub.run_for(def.id, a).unwrap().state, RunState::Firing);
        assert_eq!(hub.run_for(def.id, b).unwrap().state, RunState::Running);
    }

    #[test]
    fn start_all_resumes_client_copies_where_they_are() {
        let mut hub = hub();
        let now = Utc::now();
        let (def, [(a, mut rx_a), (b, _)]) = two_clients_one_timer(&mut hub, countdown(60));
        hub.bulk_client(a, Bulk::Start, now).unwrap();
        let later = now + chrono::Duration::seconds(10);
        hub.bulk_client(a, Bulk::Pause, later).unwrap();
        let copy = hub.run_for(def.id, a).unwrap().clone();
        assert_eq!(copy.remaining_ms, 50_000);
        drain(&mut rx_a);

        // a resumes its own copy with the 50s it had left; b starts the shared run from the top.
        let resume = later + chrono::Duration::seconds(30);
        hub.bulk_all(Bulk::Start, resume).unwrap();
        let ra = hub.run_for(def.id, a).unwrap().clone();
        assert!(hub.detached.contains_key(&(def.id, a)), "a keeps its own copy");
        assert_eq!(ra.run_id, copy.run_id, "resumed, not restarted");
        assert_eq!(ra.ends_at, Some(resume + chrono::Duration::seconds(50)));
        assert_eq!(hub.run_for(def.id, b), hub.runs.get(&def.id));
        assert_eq!(hub.runs[&def.id].ends_at, Some(resume + chrono::Duration::seconds(60)));
        assert!(
            drain(&mut rx_a).iter().all(|m| !matches!(m, ServerMsg::RunUpdate { run, .. } if run.run_id != ra.run_id)),
            "a is never sent the shared run"
        );

        // Everything is running: nothing left to start.
        assert!(hub.bulk_all(Bulk::Start, resume).is_err());

        // Only a reset brings a back onto the shared run.
        hub.bulk_all(Bulk::Reset, resume).unwrap();
        assert!(hub.detached.is_empty());
    }

    #[test]
    fn timer_card_start_and_pause_keep_copies_but_reset_resyncs() {
        let mut hub = hub();
        let now = Utc::now();
        let (def, [(a, _), (b, _)]) = two_clients_one_timer(&mut hub, countdown(60));
        hub.bulk_client(a, Bulk::Start, now).unwrap();
        hub.bulk_client(a, Bulk::Pause, now).unwrap();

        hub.control(def.id, Control::Start, now).unwrap();
        assert_eq!(hub.runs[&def.id].state, RunState::Running);
        assert_eq!(hub.run_for(def.id, a).unwrap().state, RunState::Running);
        assert!(hub.detached.contains_key(&(def.id, a)), "a keeps its own copy");

        hub.bulk_client(b, Bulk::Pause, now).unwrap();
        hub.control(def.id, Control::Pause, now).unwrap();
        assert_eq!(hub.runs[&def.id].state, RunState::Paused);
        assert_eq!(hub.run_for(def.id, a).unwrap().state, RunState::Paused);
        assert!(hub.detached.contains_key(&(def.id, b)), "b keeps its own paused copy");

        hub.control(def.id, Control::Reset, now).unwrap();
        assert!(hub.detached.is_empty());
        assert_eq!(hub.runs[&def.id].state, RunState::Idle);
    }

    #[test]
    fn client_reset_rejoins_idle_shared_run() {
        let mut hub = hub();
        let now = Utc::now();
        let (def, [(a, _), _]) = two_clients_one_timer(&mut hub, countdown(60));
        hub.bulk_client(a, Bulk::Start, now).unwrap();
        hub.bulk_client(a, Bulk::Reset, now).unwrap();
        assert!(hub.detached.is_empty());

        // While the shared run is going, a reset copy stays idle on its own.
        hub.control(def.id, Control::Start, now).unwrap();
        hub.bulk_client(a, Bulk::Reset, now).unwrap();
        assert_eq!(hub.run_for(def.id, a).unwrap().state, RunState::Idle);
        assert_eq!(hub.runs[&def.id].state, RunState::Running);
    }

    #[test]
    fn client_dismiss_of_own_copy_leaves_others_firing() {
        let mut hub = hub();
        let now = Utc::now();
        let (def, [(a, _), (b, _)]) = two_clients_one_timer(&mut hub, countdown(60));
        hub.bulk_client(a, Bulk::Start, now).unwrap();
        hub.bulk_client(b, Bulk::Start, now).unwrap();
        hub.tick(now + chrono::Duration::seconds(61));

        hub.agent_dismiss(a, def.id, now);
        assert_eq!(hub.run_for(def.id, a).unwrap().state, RunState::Idle);
        assert!(!hub.detached.contains_key(&(def.id, a)), "a rejoins the idle shared run");
        assert_eq!(hub.run_for(def.id, b).unwrap().state, RunState::Firing);
    }

    #[test]
    fn non_dismissable_copy_idles_when_its_client_completes() {
        let mut hub = hub();
        let now = Utc::now();
        let def = TimerDef { allow_client_dismiss: false, ..countdown(60) };
        let (def, [(a, _), (b, _)]) = two_clients_one_timer(&mut hub, def);
        hub.bulk_client(a, Bulk::Start, now).unwrap();
        hub.bulk_client(b, Bulk::Start, now).unwrap();
        hub.tick(now + chrono::Duration::seconds(61));

        let run_a = hub.run_for(def.id, a).unwrap().run_id;
        hub.agent_run_complete(a, def.id, run_a, now);
        assert!(!hub.detached.contains_key(&(def.id, a)));
        assert_eq!(hub.run_for(def.id, b).unwrap().state, RunState::Firing, "still waiting on b");
    }

    #[test]
    fn client_dismiss_from_deck_leaves_shared_run_firing_for_others() {
        let mut hub = hub();
        let now = Utc::now();
        let (def, [(a, mut rx_a), (b, mut rx_b)]) = two_clients_one_timer(&mut hub, countdown(60));
        hub.control(def.id, Control::Start, now).unwrap();
        hub.tick(now + chrono::Duration::seconds(61));
        drain(&mut rx_a);
        drain(&mut rx_b);

        let run = hub.control_client(a, def.id, Control::Dismiss, now).unwrap();
        assert_eq!(run.state, RunState::Idle);
        assert!(hub.detached.contains_key(&(def.id, a)), "a waits on its own idle copy");
        assert_eq!(hub.run_for(def.id, b).unwrap().state, RunState::Firing);
        assert_eq!(hub.runs[&def.id].state, RunState::Firing);
        assert!(
            drain(&mut rx_a)
                .iter()
                .any(|m| matches!(m, ServerMsg::RunUpdate { run, .. } if run.state == RunState::Idle))
        );
        assert!(drain(&mut rx_b).is_empty(), "b is not told about a's dismissal");

        // Nothing to dismiss now.
        assert!(hub.control_client(a, def.id, Control::Dismiss, now).is_err());
        // Once the shared run is dismissed too, a rejoins it.
        hub.control(def.id, Control::Dismiss, now).unwrap();
        assert!(hub.detached.is_empty());
    }

    #[test]
    fn client_add_extends_only_that_client() {
        let mut hub = hub();
        let now = Utc::now();
        let (def, [(a, _), (b, _)]) = two_clients_one_timer(&mut hub, countdown(60));
        hub.control(def.id, Control::Start, now).unwrap();
        let shared_end = hub.runs[&def.id].ends_at;

        hub.control_client(a, def.id, Control::Add { secs: 60 }, now).unwrap();
        assert_eq!(hub.run_for(def.id, a).unwrap().ends_at, Some(now + chrono::Duration::seconds(120)));
        assert_eq!(hub.run_for(def.id, b).unwrap().ends_at, shared_end);

        // Adding again extends a's existing copy.
        hub.control_client(a, def.id, Control::Add { secs: 60 }, now).unwrap();
        assert_eq!(hub.run_for(def.id, a).unwrap().ends_at, Some(now + chrono::Duration::seconds(180)));
    }

    #[test]
    fn client_control_rejects_scheduled_add_unless_firing_and_unknown_targets() {
        let mut hub = hub();
        let now = Utc::now();
        let scheduled = TimerDef {
            kind: TimerKind::AtTime { time: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(), repeat: Repeat::Daily },
            ..countdown(1)
        };
        let (def, [(a, _), (b, _)]) = two_clients_one_timer(&mut hub, scheduled);
        assert!(hub.control_client(a, def.id, Control::Add { secs: 60 }, now).is_err());
        assert!(hub.detached.is_empty(), "failed op leaves no copy");

        let other = hub.upsert_timer(countdown(60), now);
        assert!(hub.control_client(a, other.id, Control::Dismiss, now).is_err(), "not assigned");
        assert!(hub.control_client(Uuid::new_v4(), def.id, Control::Dismiss, now).is_err(), "unknown client");
        assert!(hub.control_client(b, Uuid::new_v4(), Control::Dismiss, now).is_err(), "unknown timer");
    }

    #[test]
    fn unassigning_or_removing_drops_client_copies() {
        let mut hub = hub();
        let now = Utc::now();
        let (def, [(a, _), (b, _)]) = two_clients_one_timer(&mut hub, countdown(60));
        hub.bulk_client(a, Bulk::Start, now).unwrap();
        hub.bulk_client(b, Bulk::Start, now).unwrap();

        hub.set_assignments(a, vec![], now).unwrap();
        assert!(!hub.detached.contains_key(&(def.id, a)));
        hub.remove_client(b, "gone").unwrap();
        assert!(hub.detached.is_empty());
    }

    #[test]
    fn start_all_client_timers_covers_every_approved_client_once() {
        let mut hub = hub();
        let now = Utc::now();
        let shared = hub.upsert_timer(countdown(60), now);
        let only_b = hub.upsert_timer(countdown(60), now);
        let pending_only = hub.upsert_timer(countdown(60), now);
        let unassigned = hub.upsert_timer(countdown(60), now);
        let (a, _) = hello(&mut hub, None, None).unwrap();
        let (b, _) = hello(&mut hub, None, None).unwrap();
        let (p, _) = hello(&mut hub, None, None).unwrap();
        hub.approve_client(a, now).unwrap();
        hub.approve_client(b, now).unwrap();
        hub.set_assignments(a, vec![shared.id], now).unwrap();
        hub.set_assignments(b, vec![shared.id, only_b.id], now).unwrap();
        hub.set_assignments(p, vec![pending_only.id], now).unwrap();

        let started = hub.bulk_all(Bulk::Start, now).unwrap();
        assert_eq!(started.len(), 2, "shared timer started once");
        assert_eq!(hub.runs[&shared.id].state, RunState::Running);
        assert_eq!(hub.runs[&only_b.id].state, RunState::Running);
        assert_eq!(hub.runs[&pending_only.id].state, RunState::Idle, "pending client ignored");
        assert_eq!(hub.runs[&unassigned.id].state, RunState::Idle);
        assert!(hub.bulk_all(Bulk::Start, now).is_err(), "nothing left to start");
    }

    #[test]
    fn bulk_pause_and_reset() {
        let mut hub = hub();
        let now = Utc::now();
        let a_timer = hub.upsert_timer(countdown(60), now);
        let b_timer = hub.upsert_timer(countdown(60), now);
        let scheduled = hub.upsert_timer(
            TimerDef {
                kind: TimerKind::AtTime {
                    time: chrono::NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
                    repeat: Repeat::Daily,
                },
                ..countdown(1)
            },
            now,
        );
        let (a, _) = hello(&mut hub, None, None).unwrap();
        let (b, _) = hello(&mut hub, None, None).unwrap();
        hub.approve_client(a, now).unwrap();
        hub.approve_client(b, now).unwrap();
        hub.set_assignments(a, vec![a_timer.id, scheduled.id], now).unwrap();
        hub.set_assignments(b, vec![b_timer.id], now).unwrap();
        hub.bulk_all(Bulk::Start, now).unwrap();

        // Per client: only that client's countdowns; the scheduled timer keeps running.
        let paused = hub.bulk_client(a, Bulk::Pause, now).unwrap();
        assert_eq!(paused.len(), 1);
        assert_eq!(hub.run_for(a_timer.id, a).unwrap().state, RunState::Paused);
        assert_eq!(hub.run_for(scheduled.id, a).unwrap().state, RunState::Running);
        assert_eq!(hub.run_for(b_timer.id, b).unwrap().state, RunState::Running);
        assert!(hub.bulk_client(a, Bulk::Pause, now).is_err(), "nothing left to pause");

        hub.bulk_all(Bulk::Pause, now).unwrap();
        assert_eq!(hub.runs[&b_timer.id].state, RunState::Paused);
        assert_eq!(hub.run_for(a_timer.id, a).unwrap().state, RunState::Paused);

        let reset = hub.bulk_all(Bulk::Reset, now).unwrap();
        assert_eq!(reset.len(), 3);
        assert!(hub.detached.is_empty());
        for t in [a_timer.id, b_timer.id, scheduled.id] {
            assert_eq!(hub.runs[&t].state, RunState::Idle);
        }
        assert!(hub.bulk_all(Bulk::Reset, now).is_err(), "nothing left to reset");
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

    fn hello_as(hub: &mut Hub, name: &str, enroll_key: Option<&str>) -> (ClientId, Vec<ServerMsg>) {
        match hub.agent_hello(None, None, "host".into(), name.into(), enroll_key.map(Into::into), Utc::now()) {
            HelloOutcome::Accepted { client_id, mut rx, .. } => {
                let mut msgs = vec![];
                while let Ok(m) = rx.try_recv() {
                    msgs.push(m);
                }
                (client_id, msgs)
            }
            HelloOutcome::Rejected(r) => panic!("rejected: {r}"),
        }
    }

    fn declared(json: &str) -> Resolved {
        serde_json::from_str::<crate::config::DeclaredConfig>(json).unwrap().resolve().unwrap()
    }

    #[test]
    fn enrollment_key_approves_and_issues_token() {
        let mut hub = hub();
        hub.set_enroll_key(Some("sekrit".into()));

        let (good, msgs) = hello_as(&mut hub, "a", Some("sekrit"));
        assert_eq!(hub.store.client(good).unwrap().status, ClientStatus::Approved);
        assert!(
            msgs.iter().any(|m| matches!(m, ServerMsg::Welcome { status: ClientStatus::Approved, token: Some(_), .. })),
            "token issued without a manual approval"
        );

        let (bad, _) = hello_as(&mut hub, "b", Some("wrong"));
        assert_eq!(hub.store.client(bad).unwrap().status, ClientStatus::Pending);
        let (none, _) = hello_as(&mut hub, "c", None);
        assert_eq!(hub.store.client(none).unwrap().status, ClientStatus::Pending);

        // A pending client reconnecting with the key is promoted.
        let HelloOutcome::Accepted { .. } =
            hub.agent_hello(Some(none), None, "host".into(), "c".into(), Some("sekrit".into()), Utc::now())
        else {
            panic!("rejected")
        };
        assert_eq!(hub.store.client(none).unwrap().status, ClientStatus::Approved);
    }

    #[test]
    fn enrollment_disabled_without_server_key() {
        let mut hub = hub();
        let (id, _) = hello_as(&mut hub, "a", Some("anything"));
        assert_eq!(hub.store.client(id).unwrap().status, ClientStatus::Pending);
    }

    #[test]
    fn config_manages_timers_but_not_assignments() {
        let mut hub = hub();
        let now = Utc::now();
        hub.set_enroll_key(Some("k".into()));
        let (desk, msgs) = hello_as(&mut hub, "Front desk", Some("k"));
        let token = msgs
            .iter()
            .find_map(|m| match m {
                ServerMsg::Welcome { token: Some(t), .. } => Some(t.clone()),
                _ => None,
            })
            .unwrap();
        let deck_timer = hub.upsert_timer(countdown(30), now);

        hub.apply_config(
            declared(
                r#"{"timers":[
                    {"key":"lab","name":"Lab","kind":{"type":"countdown","duration_secs":60}},
                    {"key":"old","name":"Old","kind":{"type":"countdown","duration_secs":60}}]}"#,
            ),
            now,
        );
        let lab = crate::config::timer_id("lab");
        let old = crate::config::timer_id("old");
        assert!(hub.store.client(desk).unwrap().assigned_timers.is_empty(), "config assigns nothing");

        // Assignments are made in the deck, declared timers included.
        hub.set_assignments(desk, vec![deck_timer.id, lab, old], now).unwrap();

        // Re-applying overwrites deck edits to a declared timer and deletes ones no longer declared
        // (which also drops them from clients), but leaves the rest of the assignments alone.
        hub.upsert_timer(TimerDef { name: "Edited in deck".into(), ..hub.store.timer(lab).unwrap().clone() }, now);
        hub.apply_config(
            declared(r#"{"timers":[{"key":"lab","name":"Lab","kind":{"type":"countdown","duration_secs":60}}]}"#),
            now,
        );
        assert_eq!(hub.store.timer(lab).unwrap().name, "Lab");
        assert!(hub.store.timer(old).is_none());
        assert!(hub.store.timer(deck_timer.id).is_some());
        assert_eq!(hub.store.client(desk).unwrap().assigned_timers, vec![deck_timer.id, lab]);

        // Unassigning a declared timer in the deck sticks across reconnects and re-applies.
        hub.set_assignments(desk, vec![deck_timer.id], now).unwrap();
        let HelloOutcome::Accepted { .. } =
            hub.agent_hello(Some(desk), Some(token), "host".into(), "Front desk".into(), None, now)
        else {
            panic!("reconnect rejected")
        };
        hub.apply_config(
            declared(r#"{"timers":[{"key":"lab","name":"Lab","kind":{"type":"countdown","duration_secs":60}}]}"#),
            now,
        );
        assert_eq!(hub.store.client(desk).unwrap().assigned_timers, vec![deck_timer.id]);
    }
}
