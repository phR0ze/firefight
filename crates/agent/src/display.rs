//! Keeps the client display app running for as long as the agent runs.

use std::io;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// Gives the desktop's tray host time to come up at login, or the tray icon is missed.
const START_DELAY: Duration = Duration::from_secs(5);
/// A display that ran this long was healthy, so the next restart is quick again.
const HEALTHY_RUN: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(30);

/// The client display to launch: the `firefight` app and any extra arguments for it.
#[derive(Debug, Clone)]
pub struct Display {
    pub program: PathBuf,
    pub args: Vec<String>,
}

impl Display {
    fn command(&self, ui_port: u16) -> Command {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args).arg("--mode=client").arg(format!("--agent-port={ui_port}"));
        // The display must not outlive the agent, however the agent ends.
        // SAFETY: prctl is async-signal-safe and touches no memory of the parent.
        unsafe {
            cmd.pre_exec(|| match libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) {
                -1 => Err(io::Error::last_os_error()),
                _ => Ok(()),
            });
        }
        cmd
    }
}

/// Launch the display and relaunch it whenever it exits. It is stopped when the agent exits.
pub fn supervise(display: Display, ui_port: u16) {
    // A dedicated thread that never ends: the parent-death signal fires when the thread that
    // spawned the child exits, so a short-lived thread would take the display down with it.
    std::thread::Builder::new()
        .name("firefight-display".into())
        .spawn(move || run(&display, ui_port))
        .expect("spawn display thread");
}

fn run(display: &Display, ui_port: u16) {
    std::thread::sleep(START_DELAY);
    let mut backoff = Duration::from_secs(1);
    // Named apart from `display`, which tracing's macros treat as their own helper.
    let program = display.program.display();
    loop {
        let started = Instant::now();
        tracing::info!("starting the client display {program}");
        match display.command(ui_port).status() {
            Ok(status) => tracing::warn!("client display exited ({status}); restarting it"),
            Err(e) => tracing::error!("failed to start the client display {program}: {e}"),
        }
        if started.elapsed() >= HEALTHY_RUN {
            backoff = Duration::from_secs(1);
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(MAX_BACKOFF);
    }
}
