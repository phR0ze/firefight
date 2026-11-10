//! Declarative server config (`--config`): timers and per-client-name assignments managed from
//! outside the control deck, e.g. by a NixOS module.
//!
//! Declared timers are identified by a stable `key` rather than a UUID; the timer id is derived
//! from it, so the same key always maps to the same timer across restarts and machines.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result, bail};
use firefight_core::model::{
    Action, ActionKind, ActionMode, TimerDef, TimerId, TimerKind, default_allow_client_dismiss,
};
use serde::Deserialize;
use uuid::Uuid;

/// Namespace for deriving timer ids from config keys. Fixed forever: changing it would orphan
/// every declared timer.
const TIMER_NAMESPACE: Uuid = Uuid::from_u128(0x6a1f_0c2e_93d4_4b8e_9f21_5c7a_d0e4_b3f6);

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredConfig {
    #[serde(default)]
    pub timers: Vec<DeclaredTimer>,
    /// Client name (as the agent reports it) -> timer keys assigned to it.
    #[serde(default)]
    pub assignments: HashMap<String, Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredTimer {
    pub key: String,
    pub name: String,
    pub kind: TimerKind,
    #[serde(default)]
    pub actions: Vec<DeclaredAction>,
    #[serde(default = "default_allow_client_dismiss")]
    pub allow_client_dismiss: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredAction {
    pub kind: ActionKind,
    #[serde(default)]
    pub mode: ActionMode,
}

/// The config resolved to concrete timer definitions and ids.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Resolved {
    pub timers: Vec<TimerDef>,
    pub assignments: HashMap<String, Vec<TimerId>>,
}

pub fn timer_id(key: &str) -> TimerId {
    Uuid::new_v5(&TIMER_NAMESPACE, key.as_bytes())
}

impl DeclaredConfig {
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))
    }

    /// Validate and turn keys into ids.
    pub fn resolve(self) -> Result<Resolved> {
        let mut timers = Vec::with_capacity(self.timers.len());
        for t in self.timers {
            if t.key.trim().is_empty() {
                bail!("declared timer has an empty key");
            }
            let id = timer_id(&t.key);
            if timers.iter().any(|d: &TimerDef| d.id == id) {
                bail!("timer key {:?} is declared twice", t.key);
            }
            let def = TimerDef {
                id,
                name: t.name.trim().to_string(),
                kind: t.kind,
                actions: t
                    .actions
                    .into_iter()
                    .enumerate()
                    .map(|(i, a)| Action { id: Uuid::new_v5(&id, &i.to_le_bytes()), kind: a.kind, mode: a.mode })
                    .collect(),
                allow_client_dismiss: t.allow_client_dismiss,
            };
            def.validate().map_err(|e| anyhow::anyhow!("timer {:?}: {e}", t.key))?;
            timers.push(def);
        }
        let mut assignments = HashMap::new();
        for (client, keys) in self.assignments {
            let mut ids = Vec::with_capacity(keys.len());
            for key in keys {
                let id = timer_id(&key);
                if !timers.iter().any(|d| d.id == id) {
                    bail!("assignment for {client:?} names undeclared timer {key:?}");
                }
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            assignments.insert(client, ids);
        }
        Ok(Resolved { timers, assignments })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(json: &str) -> Result<Resolved> {
        serde_json::from_str::<DeclaredConfig>(json)?.resolve()
    }

    #[test]
    fn resolves_stable_ids_and_defaults() {
        let r = parse(
            r#"{"timers":[{"key":"lab","name":"Lab closes","kind":{"type":"countdown","duration_secs":900},
                "actions":[{"kind":{"type":"command","cmd":"true"}}]}],
                "assignments":{"Front desk":["lab","lab"]}}"#,
        )
        .unwrap();
        let t = &r.timers[0];
        assert_eq!(t.id, timer_id("lab"));
        assert_eq!(t.id, timer_id("lab"), "deterministic");
        assert!(t.allow_client_dismiss);
        assert_eq!(t.actions[0].mode, ActionMode::Parallel);
        assert_eq!(r.assignments["Front desk"], vec![t.id], "duplicates collapsed");
        // Action ids are stable too.
        assert_eq!(
            parse(
                r#"{"timers":[{"key":"lab","name":"x","kind":{"type":"countdown","duration_secs":1},
            "actions":[{"kind":{"type":"command","cmd":"y"}}]}]}"#
            )
            .unwrap()
            .timers[0]
                .actions[0]
                .id,
            t.actions[0].id
        );
    }

    #[test]
    fn rejects_bad_config() {
        let dup = r#"{"timers":[{"key":"a","name":"a","kind":{"type":"countdown","duration_secs":1}},
                                {"key":"a","name":"b","kind":{"type":"countdown","duration_secs":1}}]}"#;
        assert!(parse(dup).unwrap_err().to_string().contains("declared twice"));
        let zero = r#"{"timers":[{"key":"a","name":"a","kind":{"type":"countdown","duration_secs":0}}]}"#;
        assert!(parse(zero).unwrap_err().to_string().contains("duration"));
        let missing = r#"{"assignments":{"c":["nope"]}}"#;
        assert!(parse(missing).unwrap_err().to_string().contains("undeclared"));
        assert!(serde_json::from_str::<DeclaredConfig>(r#"{"timerz":[]}"#).is_err(), "typos are errors");
    }
}
