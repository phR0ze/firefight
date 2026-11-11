//! The agent's mirror of its assigned timers, plus the local firing decision.

use std::collections::{HashMap, VecDeque};

use chrono::{DateTime, Duration, Utc};
use firefight_core::model::{Action, ClientId, RunId, TimerDef, TimerId};
use firefight_core::protocol::{AgentStatus, UiEvent};
use firefight_core::run::{RunState, TimerRun};

/// How many fired run ids to remember for de-duplication.
const FIRED_MEMORY: usize = 256;

/// A run that should execute its actions now.
#[derive(Debug, Clone, PartialEq)]
pub struct Fire {
    pub timer_id: TimerId,
    pub run_id: RunId,
    pub actions: Vec<Action>,
}

#[derive(Debug)]
pub struct AgentState {
    pub status: AgentStatus,
    pub client_id: Option<ClientId>,
    pub name: String,
    timers: Vec<TimerDef>,
    runs: HashMap<TimerId, TimerRun>,
    /// Estimated `server clock - local clock`, in milliseconds.
    offset_ms: Option<f64>,
    fired: VecDeque<RunId>,
    /// Runs this agent started executing, until the server reports they ended.
    executing: HashMap<TimerId, RunId>,
}

impl AgentState {
    pub fn new(name: String, client_id: Option<ClientId>) -> Self {
        Self {
            status: AgentStatus::Disconnected,
            client_id,
            name,
            timers: vec![],
            runs: HashMap::new(),
            offset_ms: None,
            fired: VecDeque::new(),
            executing: HashMap::new(),
        }
    }

    /// Fold a server timestamp into the clock offset estimate.
    pub fn observe_clock(&mut self, server_now: DateTime<Utc>, local_now: DateTime<Utc>) {
        let sample = (server_now - local_now).num_milliseconds() as f64;
        self.offset_ms = Some(match self.offset_ms {
            Some(prev) => prev * 0.8 + sample * 0.2,
            None => sample,
        });
    }

    pub fn server_now(&self, local_now: DateTime<Utc>) -> DateTime<Utc> {
        local_now + Duration::milliseconds(self.offset_ms.unwrap_or(0.0) as i64)
    }

    fn has_fired(&self, run_id: RunId) -> bool {
        self.fired.contains(&run_id)
    }

    fn mark_fired(&mut self, run_id: RunId) {
        if self.fired.len() >= FIRED_MEMORY {
            self.fired.pop_front();
        }
        self.fired.push_back(run_id);
    }

    fn fire_for(&mut self, run: &TimerRun) -> Option<Fire> {
        if self.has_fired(run.run_id) {
            return None;
        }
        let def = self.timers.iter().find(|t| t.id == run.timer_id)?;
        let fire = Fire { timer_id: def.id, run_id: run.run_id, actions: def.actions.clone() };
        self.mark_fired(run.run_id);
        self.executing.insert(run.timer_id, run.run_id);
        Some(fire)
    }

    /// Whether the run this agent is executing for `timer_id` was reset, dismissed,
    /// snoozed or unassigned since it fired. If so it is forgotten, and the caller
    /// should stop its actions and sounds.
    fn take_ended(&mut self, timer_id: TimerId) -> bool {
        let Some(run_id) = self.executing.get(&timer_id) else { return false };
        let live = self
            .runs
            .get(&timer_id)
            .is_some_and(|r| r.run_id == *run_id && matches!(r.state, RunState::Running | RunState::Firing));
        if !live {
            self.executing.remove(&timer_id);
        }
        !live
    }

    /// The server stopped `run_id`; returns whether this agent is executing it.
    pub fn take_stopped(&mut self, timer_id: TimerId, run_id: RunId) -> bool {
        let ours = self.executing.get(&timer_id) == Some(&run_id);
        if ours {
            self.executing.remove(&timer_id);
        }
        ours
    }

    /// Replace the assigned timers. Runs that already fired before this sync
    /// (e.g. while the agent was offline) are not replayed. Returns the timers
    /// whose executing run ended meanwhile.
    pub fn apply_assigned(&mut self, timers: Vec<TimerDef>, runs: Vec<TimerRun>) -> Vec<TimerId> {
        self.timers = timers;
        self.runs = runs.into_iter().map(|r| (r.timer_id, r)).collect();
        let already: Vec<_> = self.runs.values().filter(|r| r.state == RunState::Firing).map(|r| r.run_id).collect();
        for id in already {
            if !self.has_fired(id) {
                self.mark_fired(id);
            }
        }
        let executing: Vec<_> = self.executing.keys().copied().collect();
        executing.into_iter().filter(|t| self.take_ended(*t)).collect()
    }

    /// Apply a live run update. Returns a fire if the server reports a firing run
    /// the agent has not executed yet (it was late locally), and whether the run
    /// the agent was executing ended (so its actions and sounds should stop
    /// before any new fire starts).
    pub fn apply_run(&mut self, run: TimerRun) -> (Option<Fire>, bool) {
        let timer_id = run.timer_id;
        if !self.timers.iter().any(|t| t.id == timer_id) {
            return (None, false);
        }
        let firing = run.state == RunState::Firing;
        self.runs.insert(timer_id, run.clone());
        let stopped = self.take_ended(timer_id);
        let fire = if firing { self.fire_for(&run) } else { None };
        (fire, stopped)
    }

    /// Runs whose deadline has passed by the local estimate of server time.
    pub fn due(&mut self, local_now: DateTime<Utc>) -> Vec<Fire> {
        let now = self.server_now(local_now);
        let due: Vec<_> = self.runs.values().filter(|r| r.is_due(now)).cloned().collect();
        due.iter().filter_map(|r| self.fire_for(r)).collect()
    }

    pub fn ui_event(&self, local_now: DateTime<Utc>) -> UiEvent {
        UiEvent::State {
            status: self.status,
            client_id: self.client_id,
            name: self.name.clone(),
            timers: self.timers.clone(),
            runs: self.timers.iter().filter_map(|t| self.runs.get(&t.id).cloned()).collect(),
            server_now: self.server_now(local_now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use firefight_core::model::{ActionKind, ActionMode, TimerKind};
    use uuid::Uuid;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
    }

    fn setup() -> (AgentState, TimerRun) {
        let def = TimerDef {
            id: Uuid::new_v4(),
            name: "t".into(),
            kind: TimerKind::Countdown { duration_secs: 10 },
            actions: vec![Action {
                id: Uuid::new_v4(),
                kind: ActionKind::Command { cmd: "true".into() },
                mode: ActionMode::Parallel,
            }],
            focus_on_fire: true,
        };
        let mut run = TimerRun::idle(def.id, 10_000);
        run.start(t(0)).unwrap();
        let mut s = AgentState::new("a".into(), None);
        s.apply_assigned(vec![def], vec![run.clone()]);
        (s, run)
    }

    #[test]
    fn fires_locally_once_without_server() {
        let (mut s, run) = setup();
        assert!(s.due(t(9)).is_empty());
        let fires = s.due(t(10));
        assert_eq!(fires.len(), 1);
        assert_eq!(fires[0].run_id, run.run_id);
        assert!(s.due(t(11)).is_empty(), "same run never fires twice");
        // The server's later Firing update is de-duplicated too.
        let mut firing = run.clone();
        firing.fire().unwrap();
        assert_eq!(s.apply_run(firing).0, None);
    }

    #[test]
    fn clock_offset_shifts_deadline() {
        let (mut s, _) = setup();
        // Local clock is 5s behind the server.
        s.observe_clock(t(5), t(0));
        assert_eq!(s.due(t(5)).len(), 1);
    }

    #[test]
    fn late_agent_fires_on_server_firing_update() {
        let (mut s, mut run) = setup();
        run.fire().unwrap();
        let (fire, stopped) = s.apply_run(run.clone());
        assert!(fire.is_some());
        assert!(!stopped);
        run.dismiss(10_000).unwrap();
        assert_eq!(s.apply_run(run), (None, true));
    }

    #[test]
    fn reset_after_local_fire_stops_the_run() {
        let (mut s, mut run) = setup();
        assert_eq!(s.due(t(10)).len(), 1);
        // The server resets the run before its own tick saw it fire.
        run.reset(10_000);
        assert_eq!(s.apply_run(run.clone()), (None, true));
        assert_eq!(s.apply_run(run), (None, false), "already stopped");
    }

    #[test]
    fn stop_message_only_stops_the_run_it_names() {
        let (mut s, run) = setup();
        assert_eq!(s.due(t(10)).len(), 1);
        assert!(!s.take_stopped(run.timer_id, Uuid::new_v4()));
        assert!(s.take_stopped(run.timer_id, run.run_id));
        assert!(!s.take_stopped(run.timer_id, run.run_id));
    }

    #[test]
    fn unassigning_stops_the_run() {
        let (mut s, _) = setup();
        let fired = s.due(t(10));
        assert_eq!(s.apply_assigned(vec![], vec![]), vec![fired[0].timer_id]);
    }

    #[test]
    fn sync_does_not_replay_runs_fired_while_offline() {
        let (mut s, mut run) = setup();
        run.fire().unwrap();
        let timers = s.timers.clone();
        s.apply_assigned(timers, vec![run.clone()]);
        assert_eq!(s.apply_run(run).0, None);
    }

    #[test]
    fn unassigned_runs_are_ignored() {
        let (mut s, _) = setup();
        let mut other = TimerRun::idle(Uuid::new_v4(), 1);
        other.start(t(0)).unwrap();
        other.fire().unwrap();
        assert_eq!(s.apply_run(other), (None, false));
    }
}
