use std::net::SocketAddr;

fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let addr: SocketAddr = std::env::var("PHRONA_ADDR")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string())
        .parse()?;
    let api_key = std::env::var("PHRONA_API_KEY")
        .ok()
        .filter(|k| !k.is_empty());
    phrona_api::serve(addr, api_key).await
}
