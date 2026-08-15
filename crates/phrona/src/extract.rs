use serde::Serialize;
use serde::Serializer;

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

impl Serialize for ExtractedPage {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("ExtractedPage", 5)?;
        st.serialize_field("url", &self.url)?;
        st.serialize_field("title", &self.title)?;
        st.serialize_field("description", &self.description)?;
        st.serialize_field("text", &self.text)?;
        st.serialize_field("images", &self.images)?;
        st.end()
    }
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
        return Err(Error::unavailable("extract", resp.status().as_u16()));
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
    let description = parse::doc_attr(&doc, "meta[name=\"description\"]", "content")
        .or_else(|| parse::doc_attr(&doc, "meta[property=\"og:description\"]", "content"))
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

#[cfg(test)]
mod tests {
    use super::*;

    const HTML: &str = r#"
<!doctype html><html><head>
<title>Rust Book</title>
<meta name="description" content="Learn the Rust language">
</head><body>
<main>
<h1>Ownership</h1>
<p>Rust ownership is a set of rules that govern memory management.</p>
<p>Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris nisi ut aliquip ex ea commodo consequat.</p>
<p>Borrowing lets you use data without taking it. The borrow checker enforces these rules at compile time.</p>
<img src="https://example.com/a.png">
</main>
</body></html>"#;

    #[test]
    fn extracts_title_description_images() {
        let page = extract_from_html(HTML, "https://doc.rust-lang.org", 500, None);
        assert_eq!(page.title, "Rust Book");
        assert_eq!(page.description, "Learn the Rust language");
        assert_eq!(page.images, ["https://example.com/a.png"]);
    }

    #[test]
    fn truncates_to_max_chars() {
        let page = extract_from_html(HTML, "u", 20, None);
        assert!(page.text.chars().count() <= 20);
    }

    #[test]
    fn query_bias_excerpts() {
        let page = extract_from_html(HTML, "u", 300, Some("borrowing"));
        assert!(page.text.contains("Borrowing"));
        assert!(!page.text.contains("memory management"));
        assert!(page.text.starts_with("..."));
    }

    #[test]
    fn empty_and_tiny_html_do_not_panic() {
        let page = extract_from_html("", "u", 100, None);
        assert!(page.text.is_empty());
        let page = extract_from_html("<p>hi</p>", "u", 100, Some("q"));
        assert!(!page.text.is_empty());
    }
}
