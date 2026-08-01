use scraper::{Html, Selector};

use crate::client::HttpClient;
use crate::error::{Error, Result};
use crate::parse;

/// A readable-text extraction of a web page (AI grounding).
#[derive(Debug, Clone)]
pub struct ExtractedPage {
    pub url: String,
    pub title: String,
    pub description: String,
    pub text: String,
    pub images: Vec<String>,
}

/// Fetch and extract the main content of a page.
/// `query` optionally highlights the most relevant excerpt.
pub async fn extract(
    client: &HttpClient,
    url: &str,
    max_chars: usize,
    query: Option<&str>,
) -> Result<ExtractedPage> {
    let resp = client.get(url).await?;
    if !resp.status().is_success() {
        return Err(Error::Http(format!("extract: status {}", resp.status())));
    }
    let bytes = resp.bytes().await.map_err(Error::from)?;
    let html = String::from_utf8_lossy(&bytes).into_owned();
    Ok(extract_from_html(&html, url, max_chars, query))
}

/// Pure function: parse HTML and extract readable content.
pub fn extract_from_html(
    html: &str,
    url: &str,
    max_chars: usize,
    query: Option<&str>,
) -> ExtractedPage {
    let doc = Html::parse_document(html);
    let title = parse::doc_text(&doc, "title").unwrap_or_else(|| url.to_string());
    let description = parse::doc_text(&doc, "meta[name=\"description\"]")
        .or_else(|| parse::doc_text(&doc, "meta[property=\"og:description\"]"))
        .unwrap_or_default();

    let mut text = String::new();
    for sel_str in ["article", "main", "body"] {
        let Ok(sel) = Selector::parse(sel_str) else {
            continue;
        };
        for node in doc.select(&sel) {
            let t = collect_text(&node);
            if t.chars().count() > text.chars().count() {
                text = t;
            }
        }
        if text.chars().count() > 200 {
            break;
        }
    }
    let text = parse::collapse(&text);
    let text = match query {
        Some(q) if !q.is_empty() => parse::excerpt(&text, q, max_chars / 2),
        _ => parse::truncate(&text, max_chars),
    };

    let mut images = Vec::new();
    let img_sel = Selector::parse("img[src]").unwrap();
    for node in doc.select(&img_sel) {
        if let Some(src) = node.value().attr("src") {
            if src.starts_with("http") && images.len() < 10 {
                images.push(src.to_string());
            }
        }
    }

    ExtractedPage {
        url: url.to_string(),
        title,
        description,
        text,
        images,
    }
}

fn collect_text(node: &scraper::ElementRef) -> String {
    let mut out = String::new();
    for child in
        node.select(&Selector::parse("p, h1, h2, h3, h4, li, blockquote, pre, td").unwrap())
    {
        let t = parse::text_of(&child);
        if !t.is_empty() {
            out.push_str(&t);
            out.push('\n');
        }
    }
    if out.trim().is_empty() {
        out = node.text().collect();
    }
    out
}

/// Extract several pages in parallel (used by AI grounding endpoints).
pub async fn extract_many(
    client: &HttpClient,
    urls: &[String],
    max_chars: usize,
    query: Option<&str>,
) -> Vec<Result<ExtractedPage>> {
    let futs = urls
        .iter()
        .map(|url| async move { extract(client, url, max_chars, query).await });
    futures::future::join_all(futs).await
}
