//! Grounded search for RAG: search, then answer with cited sources.
//!
//! Run with: cargo run -p phrona-examples --bin ground -- "rust ownership"

use phrona::{ResultItem, SearchClient, SearchOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rust ownership".into());
    let client = SearchClient::new()?;
    let mut opts = SearchOptions::new(query.clone());
    opts.max_results = 8;
    let resp = client.search(opts).await?;

    let answer = resp
        .answer
        .clone()
        .unwrap_or_else(|| format!("Found {} sources for \"{}\".", resp.total, resp.query));
    println!("answer: {answer}");

    let mut shown = 0;
    for (i, item) in resp.results.iter().enumerate() {
        let (title, url, content) = match item {
            ResultItem::Web(w) => (&w.title, &w.url, w.description.as_str()),
            ResultItem::News(n) => (&n.title, &n.url, n.description.as_str()),
            ResultItem::Video(v) => (&v.title, &v.url, v.description.as_str()),
            ResultItem::Image(im) => (&im.title, &im.url, im.source.as_str()),
            ResultItem::Book(b) => (&b.title, &b.url, b.info.as_str()),
        };
        if content.trim().is_empty() {
            continue;
        }
        println!("\n{}. {title}\n   {url}\n   {content}", i + 1);
        shown += 1;
        if shown >= 5 {
            break;
        }
    }
    Ok(())
}
