//! Agent configuration and pairing credentials (`agent.toml`).

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use firefight_core::DEFAULT_AGENT_UI_PORT;
use firefight_core::model::ClientId;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    /// Server address as `host:port` or a full `ws://` URL.
    pub server: String,
    /// Display name shown in the control deck.
    pub name: String,
    #[serde(default = "default_ui_port")]
    pub ui_port: u16,
    /// Assigned by the server on first connection.
    pub client_id: Option<ClientId>,
    /// Issued by the server once the client is approved.
    pub token: Option<String>,
    /// Shared enrollment key from `--enroll-key-file`; never written to the config file.
    #[serde(skip)]
    pub enroll_key: Option<String>,
}

fn default_ui_port() -> u16 {
    DEFAULT_AGENT_UI_PORT
}

impl AgentConfig {
    pub fn agent_ws_url(&self) -> String {
        let base = self.server.trim_end_matches('/');
        if base.starts_with("ws://") || base.starts_with("wss://") {
            format!("{base}/api/agent/ws")
        } else {
            format!("ws://{base}/api/agent/ws")
        }
    }

    pub fn load(path: &Path) -> Result<Option<Self>> {
        match fs::read_to_string(path) {
            Ok(s) => Ok(Some(toml::from_str(&s).with_context(|| format!("parsing {}", path.display()))?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Atomically write the config with 0600 permissions (it holds the token).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(toml::to_string_pretty(self)?.as_bytes())?;
        f.sync_all()?;
        fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
    }
}

/// `$XDG_CONFIG_HOME/firefight/agent.toml` or `~/.config/firefight/agent.toml`.
pub fn default_config_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("firefight/agent.toml")
}

pub fn hostname() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "unknown".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(server: &str) -> AgentConfig {
        AgentConfig {
            server: server.into(),
            name: "n".into(),
            ui_port: 1,
            client_id: None,
            token: None,
            enroll_key: None,
        }
    }

    #[test]
    fn ws_url_from_host_port_or_url() {
        assert_eq!(cfg("server-host:7878").agent_ws_url(), "ws://server-host:7878/api/agent/ws");
        assert_eq!(cfg("wss://server-host/").agent_ws_url(), "wss://server-host/api/agent/ws");
    }
}
