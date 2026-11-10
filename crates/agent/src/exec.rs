//! Executes a timer's actions on this machine and reports the results.

use std::process::Stdio;
use std::time::Duration;

use chrono::Utc;
use firefight_core::model::{Action, ActionKind, ActionMode, ActionResult, RunId, TimerId};
use tokio::process::Command;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::sound::SoundPlayer;

/// Commands still running after this long are killed.
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const MAX_MESSAGE: usize = 200;

pub struct Ctx {
    pub timer_id: Option<TimerId>,
    pub run_id: Option<RunId>,
    pub sound: SoundPlayer,
    pub results: mpsc::UnboundedSender<ActionResult>,
}

/// Run actions in order. `AfterPrevious` actions wait for the action before
/// them to finish; `Parallel` actions start immediately.
pub async fn execute(actions: Vec<Action>, ctx: Ctx) {
    let ctx = std::sync::Arc::new(ctx);
    let mut prev: Option<JoinHandle<()>> = None;
    let mut all = vec![];
    for action in actions {
        if action.mode == ActionMode::AfterPrevious
            && let Some(h) = prev.take()
        {
            let _ = h.await;
        }
        let ctx = ctx.clone();
        let h = tokio::spawn(async move { run_action(action, &ctx).await });
        if let Some(p) = prev.replace(h) {
            all.push(p);
        }
    }
    all.extend(prev);
    for h in all {
        let _ = h.await;
    }
}

async fn run_action(action: Action, ctx: &Ctx) {
    let report = |ok: bool, exit_code: Option<i32>, message: String| {
        let _ = ctx.results.send(ActionResult {
            timer_id: ctx.timer_id,
            run_id: ctx.run_id,
            action_id: action.id,
            ok,
            exit_code,
            message: truncate(message),
            at: Utc::now(),
        });
    };
    match &action.kind {
        ActionKind::Command { cmd } => {
            tracing::info!("running command: {cmd}");
            match run_command(cmd).await {
                Ok((code, msg)) => report(code == Some(0), code, msg),
                Err(e) => report(false, None, e),
            }
        }
        ActionKind::Sound { sound, volume, loop_until_dismissed } => {
            // Test runs have no timer to dismiss, so they never loop.
            let looping = *loop_until_dismissed && ctx.timer_id.is_some();
            match ctx.sound.play(ctx.timer_id, sound.clone(), *volume, looping).await {
                Ok(done) => {
                    report(true, None, format!("playing {sound}"));
                    let _ = done.await;
                }
                Err(e) => report(false, None, e),
            }
        }
    }
}

async fn run_command(cmd: &str) -> Result<(Option<i32>, String), String> {
    let child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to start: {e}"))?;
    let out = tokio::time::timeout(COMMAND_TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("timed out after {}s", COMMAND_TIMEOUT.as_secs()))?
        .map_err(|e| e.to_string())?;
    let code = out.status.code();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let msg = match stderr.lines().rev().find(|l| !l.trim().is_empty()) {
        Some(line) => line.trim().to_string(),
        None => match code {
            Some(c) => format!("exit {c}"),
            None => "killed by signal".into(),
        },
    };
    Ok((code, msg))
}

fn truncate(mut s: String) -> String {
    if s.len() > MAX_MESSAGE {
        let mut end = MAX_MESSAGE;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
        s.push('…');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn cmd(c: &str, mode: ActionMode) -> Action {
        Action { id: Uuid::new_v4(), kind: ActionKind::Command { cmd: c.into() }, mode }
    }

    async fn run(actions: Vec<Action>) -> Vec<ActionResult> {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let ctx = Ctx { timer_id: None, run_id: None, sound: SoundPlayer::spawn(), results: tx };
        execute(actions, ctx).await;
        let mut out = vec![];
        while let Ok(r) = rx.try_recv() {
            out.push(r);
        }
        out
    }

    #[tokio::test]
    async fn reports_exit_codes_and_stderr() {
        let results =
            run(vec![cmd("true", ActionMode::Parallel), cmd("echo boom >&2; exit 3", ActionMode::Parallel)]).await;
        assert_eq!(results.len(), 2);
        let failed = results.iter().find(|r| !r.ok).unwrap();
        assert_eq!((failed.exit_code, failed.message.as_str()), (Some(3), "boom"));
    }

    #[tokio::test]
    async fn after_previous_waits_for_exit() {
        let dir = std::env::temp_dir().join(format!("firefight-exec-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("seq");
        let results = run(vec![
            cmd(&format!("sleep 0.3; echo a >> {}", f.display()), ActionMode::Parallel),
            cmd(&format!("echo b >> {}", f.display()), ActionMode::AfterPrevious),
        ])
        .await;
        assert!(results.iter().all(|r| r.ok));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "a\nb\n");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[tokio::test]
    async fn unknown_sound_is_reported() {
        let a = Action {
            id: Uuid::new_v4(),
            kind: ActionKind::Sound { sound: "nope".into(), volume: 1.0, loop_until_dismissed: false },
            mode: ActionMode::Parallel,
        };
        let results = run(vec![a]).await;
        assert!(!results[0].ok);
    }

    #[test]
    fn truncates_on_char_boundary() {
        let s = truncate("é".repeat(150));
        assert!(s.ends_with('…'));
        assert!(s.len() <= MAX_MESSAGE + '…'.len_utf8());
    }
}
