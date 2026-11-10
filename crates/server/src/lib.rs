//! firefight server: stores timers, tracks clients and drives timer runs.
//! It never executes actions itself; assigned agents do.

pub mod api;
pub mod hub;
pub mod store;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
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

/// Load state and build the shared application state.
pub fn init(data_dir: &Path) -> Result<AppState> {
    let store = Store::open(data_dir)?;
    let admin_token = store::load_or_create_admin_token(data_dir)?;
    let hub = Hub::new(store, Utc::now());
    Ok(AppState { hub: Arc::new(Mutex::new(hub)), admin_token: Arc::new(admin_token) })
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
