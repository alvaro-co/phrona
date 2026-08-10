//! Page extraction for AI grounding: title, description, readable text,
//! images, and query-biased excerpts.
//!
//! Run with: cargo run -p metasearch-examples --bin extract -- <url> [query]

use metasearch::{SearchClient, extract, extract_many};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html".into());
    let query = std::env::args().nth(2);
    let client = SearchClient::new()?;

    let page = extract(client.http(), &url, 4000, query.as_deref()).await?;
    println!("title: {}", page.title);
    if !page.description.is_empty() {
        println!("description: {}", page.description);
    }
    println!("\ntext (excerpt):\n{}", page.text);
    if !page.images.is_empty() {
        println!("\nimages:\n{}", page.images.join("\n"));
    }

    // Several pages in parallel.
    let urls: Vec<String> = [
        "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
        "https://doc.rust-lang.org/book/ch04-02-references-and-borrowing.html",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let pages = extract_many(client.http(), &urls, 1500, None).await;
    println!("\nparallel extraction: {}", pages.len());
    for p in pages.iter().flatten() {
        println!("  {} ({} chars)", p.title, p.text.chars().count());
    }
    Ok(())
}
