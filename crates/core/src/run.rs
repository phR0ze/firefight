//! Runtime state of a timer. The server owns it; agents and the deck mirror it.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::model::{RunId, TimerId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Idle,
    Running,
    Paused,
    Firing,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerRun {
    /// Changes every time the timer is (re)started, so agents fire each run once.
    pub run_id: RunId,
    pub timer_id: TimerId,
    pub state: RunState,
    /// Set while `Running`.
    pub ends_at: Option<DateTime<Utc>>,
    /// Remaining time while `Idle`/`Paused`; 0 while `Firing`.
    pub remaining_ms: u64,
    /// Length of the current run, for progress display.
    pub total_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTransition {
    pub from: RunState,
    pub op: &'static str,
}

impl std::fmt::Display for InvalidTransition {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "cannot {} a timer that is {:?}", self.op, self.from)
    }
}

impl std::error::Error for InvalidTransition {}

type Result<T = ()> = std::result::Result<T, InvalidTransition>;

fn ms(d: Duration) -> u64 {
    d.num_milliseconds().max(0) as u64
}

impl TimerRun {
    pub fn idle(timer_id: TimerId, total_ms: u64) -> Self {
        Self {
            run_id: Uuid::new_v4(),
            timer_id,
            state: RunState::Idle,
            ends_at: None,
            remaining_ms: total_ms,
            total_ms,
        }
    }

    fn err(&self, op: &'static str) -> InvalidTransition {
        InvalidTransition { from: self.state, op }
    }

    /// Milliseconds left at `now`.
    pub fn remaining_at(&self, now: DateTime<Utc>) -> u64 {
        match (self.state, self.ends_at) {
            (RunState::Running, Some(end)) => ms(end - now),
            _ => self.remaining_ms,
        }
    }

    pub fn is_due(&self, now: DateTime<Utc>) -> bool {
        self.state == RunState::Running && self.ends_at.is_some_and(|end| end <= now)
    }

    /// Idle -> Running (new run) or Paused -> Running (same run).
    pub fn start(&mut self, now: DateTime<Utc>) -> Result {
        match self.state {
            RunState::Idle => self.run_id = Uuid::new_v4(),
            RunState::Paused => {}
            _ => return Err(self.err("start")),
        }
        self.state = RunState::Running;
        self.ends_at = Some(now + Duration::milliseconds(self.remaining_ms as i64));
        Ok(())
    }

    /// Start a new run that ends at a fixed instant (used for `AtTime` timers).
    pub fn arm(&mut self, ends_at: DateTime<Utc>, now: DateTime<Utc>) {
        self.run_id = Uuid::new_v4();
        self.state = RunState::Running;
        self.ends_at = Some(ends_at);
        self.total_ms = ms(ends_at - now);
        self.remaining_ms = self.total_ms;
    }

    pub fn pause(&mut self, now: DateTime<Utc>) -> Result {
        if self.state != RunState::Running {
            return Err(self.err("pause"));
        }
        self.remaining_ms = self.remaining_at(now);
        self.ends_at = None;
        self.state = RunState::Paused;
        Ok(())
    }

    /// Add time. On a firing timer this snoozes: it becomes a new run ending `add_ms` from now.
    pub fn add(&mut self, add_ms: u64, now: DateTime<Utc>) {
        let add = Duration::milliseconds(add_ms as i64);
        match self.state {
            RunState::Running => {
                self.ends_at = self.ends_at.map(|end| end + add);
                self.total_ms += add_ms;
            }
            RunState::Idle | RunState::Paused => {
                self.remaining_ms += add_ms;
                self.total_ms += add_ms;
            }
            RunState::Firing => {
                self.run_id = Uuid::new_v4();
                self.state = RunState::Running;
                self.ends_at = Some(now + add);
                self.total_ms = add_ms;
                self.remaining_ms = add_ms;
            }
        }
    }

    /// Running -> Firing once the deadline has passed.
    pub fn fire(&mut self) -> Result {
        if self.state != RunState::Running {
            return Err(self.err("fire"));
        }
        self.state = RunState::Firing;
        self.ends_at = None;
        self.remaining_ms = 0;
        Ok(())
    }

    /// Back to Idle with a fresh duration, from any state.
    pub fn reset(&mut self, total_ms: u64) {
        self.state = RunState::Idle;
        self.ends_at = None;
        self.remaining_ms = total_ms;
        self.total_ms = total_ms;
    }

    pub fn dismiss(&mut self, total_ms: u64) -> Result {
        if self.state != RunState::Firing {
            return Err(self.err("dismiss"));
        }
        self.reset(total_ms);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000 + secs, 0).unwrap()
    }

    fn run() -> TimerRun {
        TimerRun::idle(Uuid::new_v4(), 60_000)
    }

    #[test]
    fn start_pause_resume_keeps_remaining() {
        let mut r = run();
        r.start(t(0)).unwrap();
        let first = r.run_id;
        assert_eq!(r.remaining_at(t(15)), 45_000);
        r.pause(t(15)).unwrap();
        assert_eq!(r.remaining_at(t(100)), 45_000);
        r.start(t(100)).unwrap();
        assert_eq!(r.run_id, first, "resume keeps the run id");
        assert_eq!(r.ends_at, Some(t(145)));
    }

    #[test]
    fn fire_and_dismiss() {
        let mut r = run();
        r.start(t(0)).unwrap();
        assert!(!r.is_due(t(59)));
        assert!(r.is_due(t(60)));
        r.fire().unwrap();
        assert_eq!(r.state, RunState::Firing);
        assert!(r.start(t(61)).is_err());
        r.dismiss(60_000).unwrap();
        assert_eq!((r.state, r.remaining_ms), (RunState::Idle, 60_000));
    }

    #[test]
    fn restart_gets_new_run_id() {
        let mut r = run();
        r.start(t(0)).unwrap();
        let first = r.run_id;
        r.reset(60_000);
        r.start(t(1)).unwrap();
        assert_ne!(r.run_id, first);
    }

    #[test]
    fn add_extends_running_and_snoozes_firing() {
        let mut r = run();
        r.start(t(0)).unwrap();
        r.add(60_000, t(10));
        assert_eq!(r.ends_at, Some(t(120)));
        r.fire().unwrap();
        let fired = r.run_id;
        r.add(60_000, t(130));
        assert_eq!(r.state, RunState::Running);
        assert_eq!(r.ends_at, Some(t(190)));
        assert_ne!(r.run_id, fired, "snooze refires as a new run");
    }

    #[test]
    fn arm_sets_total_from_now() {
        let mut r = run();
        r.arm(t(300), t(0));
        assert_eq!((r.state, r.total_ms), (RunState::Running, 300_000));
    }

    #[test]
    fn invalid_transitions_error() {
        let mut r = run();
        assert!(r.pause(t(0)).is_err());
        assert!(r.fire().is_err());
        assert!(r.dismiss(1).is_err());
    }
}
