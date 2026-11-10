//! Persisted server state: timer definitions, clients and their assignments.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use firefight_core::model::{ClientId, ClientStatus, TimerDef, TimerId};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientRecord {
    pub id: ClientId,
    pub name: String,
    pub hostname: String,
    pub status: ClientStatus,
    /// SHA-256 of the agent's token. `None` until the token is issued, which
    /// happens on the first connection after approval.
    pub token_hash: Option<String>,
    pub assigned_timers: Vec<TimerId>,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Persisted {
    pub timers: Vec<TimerDef>,
    pub clients: Vec<ClientRecord>,
}

pub struct Store {
    path: PathBuf,
    pub data: Persisted,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir).with_context(|| format!("creating {}", data_dir.display()))?;
        fs::set_permissions(data_dir, fs::Permissions::from_mode(0o700))?;
        let path = data_dir.join("state.json");
        let data = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Persisted::default(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { path, data })
    }

    /// Atomically write the state to disk.
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_vec_pretty(&self.data)?;
        write_private(&self.path, &json)
    }

    pub fn timer(&self, id: TimerId) -> Option<&TimerDef> {
        self.data.timers.iter().find(|t| t.id == id)
    }

    pub fn client(&self, id: ClientId) -> Option<&ClientRecord> {
        self.data.clients.iter().find(|c| c.id == id)
    }

    pub fn client_mut(&mut self, id: ClientId) -> Option<&mut ClientRecord> {
        self.data.clients.iter_mut().find(|c| c.id == id)
    }
}

/// Write `bytes` to `path` via a 0600 temp file and rename.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let mut f = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(&tmp, path).with_context(|| format!("writing {}", path.display()))
}

/// Random 256-bit token, hex encoded.
pub fn new_token() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    hex(&bytes)
}

pub fn hash_token(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
}

/// Constant-time comparison of a presented token against a stored hash.
pub fn token_matches(token: &str, hash: &str) -> bool {
    ct_eq(&hash_token(token), hash)
}

/// Constant-time comparison of two plain tokens (compared via their hashes).
pub fn token_matches_plain(token: &str, expected: &str) -> bool {
    ct_eq(&hash_token(token), &hash_token(expected))
}

fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Load the admin token from `<data_dir>/admin_token`, creating it on first run.
pub fn load_or_create_admin_token(data_dir: &Path) -> Result<String> {
    let path = data_dir.join("admin_token");
    match fs::read_to_string(&path) {
        Ok(s) if !s.trim().is_empty() => Ok(s.trim().to_string()),
        Ok(_) | Err(_) => {
            let token = new_token();
            write_private(&path, token.as_bytes())?;
            Ok(token)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_hash_matches_only_same_token() {
        let t = new_token();
        assert_eq!(t.len(), 64);
        let h = hash_token(&t);
        assert!(token_matches(&t, &h));
        assert!(!token_matches(&new_token(), &h));
    }
}
