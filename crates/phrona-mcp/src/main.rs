#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();
    let cfg = match phrona::PhronaConfig::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!("{e}; falling back to defaults");
            phrona::PhronaConfig::defaults()
        }
    };
    phrona_mcp::run_stdio(&cfg).await
}
