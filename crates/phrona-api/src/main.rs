use std::net::SocketAddr;

use phrona::PhronaConfig;

fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cfg = match PhronaConfig::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!("{e}; falling back to defaults");
            PhronaConfig::defaults()
        }
    };
    // Legacy single-address override kept for backwards compatibility.
    let addr: SocketAddr = std::env::var("PHRONA_ADDR")
        .ok()
        .filter(|a| !a.is_empty())
        .map(|a| a.parse())
        .transpose()?
        .unwrap_or(cfg.bind_addr()?);
    phrona_api::serve(addr, cfg).await
}
