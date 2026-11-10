//! Domain model shared by the server, the agent and (via JSON) the Flutter app.

use chrono::{NaiveTime, Weekday};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type TimerId = Uuid;
pub type ClientId = Uuid;
pub type ActionId = Uuid;
pub type RunId = Uuid;

/// A reusable timer definition configured from the control deck.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimerDef {
    pub id: TimerId,
    pub name: String,
    pub kind: TimerKind,
    /// Actions executed on every assigned client when the timer finishes.
    #[serde(default)]
    pub actions: Vec<Action>,
    /// Whether client displays may dismiss the timer while it is firing.
    /// The control deck can always dismiss.
    #[serde(default = "default_allow_client_dismiss")]
    pub allow_client_dismiss: bool,
}

pub fn default_allow_client_dismiss() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TimerKind {
    /// Counts down a fixed duration once started.
    Countdown { duration_secs: u64 },
    /// Fires at a wall-clock time in the server's local timezone.
    AtTime {
        #[serde(with = "hhmm")]
        time: NaiveTime,
        repeat: Repeat,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Repeat {
    /// Fire at the next occurrence of the time, then stop.
    Once,
    Daily,
    /// Monday through Friday.
    Weekdays,
    Days {
        days: Vec<Weekday>,
    },
}

impl Repeat {
    pub fn includes(&self, day: Weekday) -> bool {
        match self {
            Repeat::Once | Repeat::Daily => true,
            Repeat::Weekdays => !matches!(day, Weekday::Sat | Weekday::Sun),
            Repeat::Days { days } => days.contains(&day),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub kind: ActionKind,
    #[serde(default)]
    pub mode: ActionMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActionKind {
    /// Play a bundled sound by name or an absolute file path on the client.
    Sound {
        sound: String,
        /// 0.0 - 1.0
        volume: f32,
        loop_until_dismissed: bool,
    },
    /// Run a shell command via `sh -c` on the client.
    Command { cmd: String },
}

/// How an action is sequenced relative to the one before it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionMode {
    /// Start immediately alongside the previous action.
    #[default]
    Parallel,
    /// Wait for the previous action to finish (command exit) before starting.
    AfterPrevious,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientStatus {
    Pending,
    Approved,
}

/// A client as presented to the control deck (no secrets).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClientView {
    pub id: ClientId,
    pub name: String,
    pub hostname: String,
    pub status: ClientStatus,
    pub online: bool,
    pub last_seen: Option<chrono::DateTime<chrono::Utc>>,
    pub assigned_timers: Vec<TimerId>,
    pub last_result: Option<ActionResult>,
}

/// Outcome of a single action executed on a client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub timer_id: Option<TimerId>,
    pub run_id: Option<RunId>,
    pub action_id: ActionId,
    pub ok: bool,
    pub exit_code: Option<i32>,
    pub message: String,
    pub at: chrono::DateTime<chrono::Utc>,
}

/// Serialize a `NaiveTime` as `HH:MM`.
mod hhmm {
    use chrono::NaiveTime;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(t: &NaiveTime, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&t.format("%H:%M").to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<NaiveTime, D::Error> {
        let s = String::deserialize(d)?;
        NaiveTime::parse_from_str(&s, "%H:%M").map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_def_json_round_trip() {
        let def = TimerDef {
            id: Uuid::new_v4(),
            name: "Stand-up".into(),
            kind: TimerKind::AtTime { time: NaiveTime::from_hms_opt(9, 45, 0).unwrap(), repeat: Repeat::Weekdays },
            actions: vec![
                Action {
                    id: Uuid::new_v4(),
                    kind: ActionKind::Sound { sound: "gong".into(), volume: 0.8, loop_until_dismissed: true },
                    mode: ActionMode::Parallel,
                },
                Action {
                    id: Uuid::new_v4(),
                    kind: ActionKind::Command { cmd: "loginctl lock-session".into() },
                    mode: ActionMode::AfterPrevious,
                },
            ],
            allow_client_dismiss: false,
        };
        let json = serde_json::to_string(&def).unwrap();
        assert!(json.contains(r#""time":"09:45""#));
        assert!(json.contains(r#""type":"at_time""#));
        assert_eq!(serde_json::from_str::<TimerDef>(&json).unwrap(), def);
    }

    #[test]
    fn timer_def_defaults_to_client_dismiss() {
        let json =
            r#"{"id":"00000000-0000-0000-0000-000000000000","name":"t","kind":{"type":"countdown","duration_secs":5}}"#;
        assert!(serde_json::from_str::<TimerDef>(json).unwrap().allow_client_dismiss);
    }

    #[test]
    fn weekdays_excludes_weekend() {
        assert!(Repeat::Weekdays.includes(Weekday::Fri));
        assert!(!Repeat::Weekdays.includes(Weekday::Sat));
        assert!(Repeat::Days { days: vec![Weekday::Sat] }.includes(Weekday::Sat));
    }
}
