//! Suggestions from every autocomplete source, single and aggregated.
//!
//! Run with: cargo run -p phrona-examples --bin suggest -- "rust"

use phrona::{SearchClient, SuggestSource, suggest, suggest_all};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let query = std::env::args().nth(1).unwrap_or_else(|| "rust".into());
    let client = SearchClient::new()?;

    // One source.
    for source in [SuggestSource::Bing, SuggestSource::Wikipedia] {
        let list = suggest(client.http(), source, &query, "us-en").await?;
        println!("{}: {}", source.name(), list.join(" | "));
    }

    // Every source in parallel (sources that fail yield empty lists).
    println!("\nall sources:");
    let all = suggest_all(client.http(), &query, "us-en").await;
    for (source, list) in all {
        println!("  {:<12} {}", source.name(), list.join(" | "));
    }
    Ok(())
}
