//! Types and logic shared by `firefight-server` and `firefight-agent`.

pub mod model;
pub mod protocol;
pub mod run;
pub mod schedule;

/// Default TCP port of the firefight server.
pub const DEFAULT_SERVER_PORT: u16 = 7878;
/// Default loopback port the agent serves the client display on.
pub const DEFAULT_AGENT_UI_PORT: u16 = 7879;
