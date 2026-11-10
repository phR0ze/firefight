use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use firefight_core::DEFAULT_SERVER_PORT;
use tokio::net::TcpListener;

const USAGE: &str = "usage: firefight-server [--bind <addr:port>] [--data-dir <path>]";

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let mut bind = format!("0.0.0.0:{DEFAULT_SERVER_PORT}");
    let mut data_dir = firefight_server::default_data_dir();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bind" => bind = args.next().context(USAGE)?,
            "--data-dir" => data_dir = PathBuf::from(args.next().context(USAGE)?),
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown argument {other}\n{USAGE}"),
        }
    }

    let state = firefight_server::init(&data_dir)?;
    let listener = TcpListener::bind(&bind).await.with_context(|| format!("binding {bind}"))?;
    tracing::info!(
        "listening on {}, admin token in {}",
        listener.local_addr()?,
        data_dir.join("admin_token").display()
    );
    firefight_server::serve(listener, state).await
}
