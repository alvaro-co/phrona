//! Basic search: sync, async and the typed result accessors.
//!
//! Run with: cargo run -p phrona-examples --bin basic -- "rust programming"
//! (query argument optional, defaults to "rust programming")

use phrona::{SearchOptions, search, search_sync};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rust programming".into());

    // Blocking, one-shot (no explicit runtime needed anywhere).
    let opts = SearchOptions::new(query.clone());
    let sync_resp = search_sync(opts)?;
    println!(
        "sync:  {} results in {} ms",
        sync_resp.total, sync_resp.elapsed_ms
    );

    // Async, one-shot.
    let opts = SearchOptions::new(query.clone());
    let async_resp = search(opts).await?;
    println!(
        "async: {} results in {} ms",
        async_resp.total, async_resp.elapsed_ms
    );

    // Async with a reusable client (connection pooling, shared tokens).
    let client = phrona::SearchClient::new()?;
    let mut opts = SearchOptions::new(query);
    opts.max_results = 8;
    let resp = client.search(opts).await?;

    println!(
        "\nquery: {} (category: {})",
        resp.query,
        resp.category.as_str()
    );
    if let Some(answer) = resp.answer.as_deref() {
        println!("answer: {answer}");
    }
    for w in resp.web().take(8) {
        println!("{}. {}  [{}]", w.position, w.title, w.engines.join("+"));
        println!("   {}", w.url);
        if !w.description.is_empty() {
            println!("   {}", w.description);
        }
    }
    println!("\nper-engine report:");
    for e in &resp.engines {
        println!("  {:16} {:6} {} results", e.name, e.status, e.results);
    }
    Ok(())
}
