use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use firefight_agent::config::{self, AgentConfig};
use firefight_core::DEFAULT_AGENT_UI_PORT;
use tokio::net::TcpListener;

const USAGE: &str =
    "usage: firefight-agent [--server <host:port>] [--name <name>] [--ui-port <port>] [--config <path>]";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let (mut server, mut name, mut ui_port) = (None, None, None);
    let mut config_path = config::default_config_path();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--server" => server = Some(args.next().context(USAGE)?),
            "--name" => name = Some(args.next().context(USAGE)?),
            "--ui-port" => ui_port = Some(args.next().context(USAGE)?.parse().context("invalid --ui-port")?),
            "--config" => config_path = PathBuf::from(args.next().context(USAGE)?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }

    // Command-line values override the config file; the result is saved back.
    let mut cfg = match AgentConfig::load(&config_path)? {
        Some(c) => c,
        None => AgentConfig {
            server: server.clone().context(format!("--server is required on first run\n{USAGE}"))?,
            name: config::hostname(),
            ui_port: DEFAULT_AGENT_UI_PORT,
            client_id: None,
            token: None,
        },
    };
    if let Some(s) = server {
        if s != cfg.server {
            // Credentials belong to a specific server.
            cfg.client_id = None;
            cfg.token = None;
        }
        cfg.server = s;
    }
    if let Some(n) = name {
        cfg.name = n;
    }
    if let Some(p) = ui_port {
        cfg.ui_port = p;
    }
    cfg.save(&config_path)?;

    let ui_addr = format!("127.0.0.1:{}", cfg.ui_port);
    let ui = TcpListener::bind(&ui_addr).await.with_context(|| format!("binding {ui_addr}"))?;
    tracing::info!("client display socket on ws://{ui_addr}, config in {}", config_path.display());
    firefight_agent::run(cfg, Some(config_path), ui).await
}
