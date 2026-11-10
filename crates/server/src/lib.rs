//! firefight server: stores timers, tracks clients and drives timer runs.
//! It never executes actions itself; assigned agents do.

pub mod api;
pub mod config;
pub mod hub;
pub mod store;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use chrono::Utc;
use tokio::net::TcpListener;

use crate::api::AppState;
use crate::hub::Hub;
use crate::store::Store;

/// Default data directory: `$XDG_DATA_HOME/firefight` or `~/.local/share/firefight`.
pub fn default_data_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("firefight")
}

/// Optional start-up inputs, typically supplied by a service manager.
#[derive(Debug, Default)]
pub struct Options {
    /// Use this admin token instead of the one generated in the data directory.
    pub admin_token_file: Option<PathBuf>,
    /// Shared key that auto-approves agents presenting it.
    pub enroll_key_file: Option<PathBuf>,
    /// Declarative timers and assignments, see [`config`].
    pub config: Option<PathBuf>,
}

/// Load state and build the shared application state.
pub fn init(data_dir: &Path) -> Result<AppState> {
    init_with(data_dir, &Options::default())
}

pub fn init_with(data_dir: &Path, opts: &Options) -> Result<AppState> {
    // Fail before touching state if the config is invalid.
    let declared = match &opts.config {
        Some(path) => {
            Some(config::DeclaredConfig::load(path)?.resolve().with_context(|| format!("in {}", path.display()))?)
        }
        None => None,
    };
    let store = Store::open(data_dir)?;
    let admin_token = match &opts.admin_token_file {
        Some(path) => read_secret(path)?,
        None => store::load_or_create_admin_token(data_dir)?,
    };
    let mut hub = Hub::new(store, Utc::now());
    hub.set_enroll_key(opts.enroll_key_file.as_deref().map(read_secret).transpose()?);
    if let Some(declared) = declared {
        hub.apply_config(declared, Utc::now());
    }
    Ok(AppState { hub: Arc::new(Mutex::new(hub)), admin_token: Arc::new(admin_token) })
}

/// Read a secret from a file, ignoring surrounding whitespace.
fn read_secret(path: &Path) -> Result<String> {
    let s = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let s = s.trim();
    if s.is_empty() {
        bail!("{} is empty", path.display());
    }
    Ok(s.to_string())
}

/// Serve the API on `listener` until the process exits.
pub async fn serve(listener: TcpListener, state: AppState) -> Result<()> {
    let hub = state.hub.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        loop {
            tick.tick().await;
            hub.lock().expect("hub mutex poisoned").tick(Utc::now());
        }
    });
    axum::serve(listener, api::router(state)).await?;
    Ok(())
}
