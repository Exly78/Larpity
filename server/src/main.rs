use std::env;

use larpity_server::Config;
use tokio::net::TcpListener;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let config = Config::from_env();
    if config.token.is_none() {
        warn!("LARPITY_TOKEN is not set: anyone can connect as the phone or the AI host");
    }
    let addr = env::var("LARPITY_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".into());
    let listener = TcpListener::bind(&addr).await?;
    info!(
        "listening on http://{}  (client: {}, phone page: {})",
        listener.local_addr()?,
        config.client_dir.display(),
        config.firmware_dir.display()
    );
    axum::serve(listener, larpity_server::app(config))
        .with_graceful_shutdown(shutdown_signal())
        .await
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut term =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("can listen for SIGTERM");
        tokio::select! {
            _ = ctrl_c => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    let _ = ctrl_c.await;
    info!("shutting down");
}
