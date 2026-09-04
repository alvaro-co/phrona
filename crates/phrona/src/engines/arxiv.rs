//! arXiv paper search engine.
//!
//! Uses the public Atom API without authentication. arXiv asks clients
//! for at most one request per 3 seconds; the engine enforces that gap
//! process-wide (a shared timestamp plus sleep) so multi-engine fan-out
//! can never violate it. Results map onto the book shape (author,
//! publisher, abstract) — see [`crate::search::to_result_item`].

use std::time::{Duration, Instant};

use async_trait::async_trait;
use quick_xml::Reader;
use quick_xml::events::Event;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Minimum gap between arXiv API requests (upstream politeness rule).
const POLITENESS_GAP: Duration = Duration::from_secs(3);

/// Last arXiv request start, process-wide across all clients/searches.
/// `parking_lot` (already a workspace dependency): no poisoning, and the
/// lock is only ever held for timestamp arithmetic, never across `.await`.
static LAST_REQUEST: parking_lot::Mutex<Option<Instant>> = parking_lot::Mutex::new(None);

/// arXiv paper search (`Category::Papers`).
pub struct ArXiv;

#[async_trait]
impl Engine for ArXiv {
    fn name(&self) -> &'static str {
        "arxiv"
    }

    fn category(&self) -> Category {
        Category::Papers
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let max = opts.max_results.clamp(1, 50);
        let start = (opts.page.max(1) as usize - 1) * max;
        let url = parse::with_query(
            "https://export.arxiv.org/api/query",
            [
                ("search_query", format!("all:{}", opts.query).as_str()),
                ("start", start.to_string().as_str()),
                ("max_results", max.to_string().as_str()),
                ("sortBy", "relevance"),
                ("sortOrder", "descending"),
            ],
        );
        // politeness: at most one request per POLITENESS_GAP, enforced
        // across concurrent searches sharing this process
        let wait = {
            let mut last = LAST_REQUEST.lock();
            let now = Instant::now();
            let wait = last
                .map(|t| POLITENESS_GAP.saturating_sub(now.duration_since(t)))
                .unwrap_or(Duration::ZERO);
            *last = Some(now + wait);
            wait
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Any)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_arxiv(&text, self.name()))
    }
}

/// One parsed Atom `<entry>`.
#[derive(Default)]
struct Entry {
    title: String,
    id: String,
    published: String,
    summary: String,
    authors: Vec<String>,
}

/// Collapse Atom's indented multi-line text into single spaces.
fn clean(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Decode a text event to display text: bytes to string, then XML
/// entities (`&lt;`, `&amp;`, ...) to their characters. Either step
/// falls back to the raw text rather than dropping the result.
fn text_of(event: quick_xml::events::BytesText<'_>) -> String {
    let decoded = event.decode().map(|c| c.into_owned()).unwrap_or_default();
    quick_xml::escape::unescape(&decoded)
        .map(|c| c.into_owned())
        .unwrap_or(decoded)
}

/// Parse an arXiv Atom feed into [`RawResult`] items (book-shaped: authors
/// in `author`, `"arXiv"` as publisher, the abstract as `info`).
pub fn parse_arxiv(xml: &str, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut entry: Option<Entry> = None;
    let mut pos = 0u32;
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"entry" => entry = Some(Entry::default()),
                b"author" if entry.is_some() => {
                    if let Some(name) = read_author(&mut reader) {
                        if let Some(en) = entry.as_mut()
                            && !name.is_empty()
                        {
                            en.authors.push(name);
                        }
                    }
                }
                b"title" | b"id" | b"published" | b"summary" if entry.is_some() => {
                    let tag = e.local_name().as_ref().to_vec();
                    if let Ok(text) = reader.read_text(e.name()) {
                        let text = clean(&text_of(text));
                        if let Some(en) = entry.as_mut() {
                            match tag.as_slice() {
                                b"title" => en.title = text,
                                b"id" => en.id = text,
                                b"published" => en.published = text,
                                _ => en.summary = text,
                            }
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::End(e)) if e.local_name().as_ref() == b"entry" => {
                if let Some(en) = entry.take()
                    && !en.title.is_empty()
                    && !en.id.is_empty()
                {
                    pos += 1;
                    // entry ids arrive as http; upgrade to https
                    let url = en
                        .id
                        .strip_prefix("http://")
                        .map(|rest| format!("https://{rest}"))
                        .unwrap_or(en.id);
                    let authors = if en.authors.len() > 8 {
                        format!("{} et al.", en.authors[..8].join(", "))
                    } else {
                        en.authors.join(", ")
                    };
                    out.push(RawResult {
                        title: en.title,
                        url,
                        description: en.summary,
                        author: authors,
                        publisher: "arXiv".to_string(),
                        published: (!en.published.is_empty()).then_some(en.published),
                        engine: engine.to_string(),
                        position: pos,
                        ..Default::default()
                    });
                }
            }
            Ok(Event::Eof) => break,
            Err(_) => break,
            _ => {}
        }
        buf.clear();
    }
    out
}

/// Read the `<name>` inside an `<author>` element; consumes through the
/// author's end tag so the outer loop stays in sync.
fn read_author(reader: &mut Reader<&[u8]>) -> Option<String> {
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) if e.local_name().as_ref() == b"name" => {
                let raw = reader.read_text(e.name()).ok()?;
                let text = text_of(raw).trim().to_string();
                // drain to </author>
                loop {
                    buf.clear();
                    match reader.read_event_into(&mut buf) {
                        Ok(Event::End(e)) if e.local_name().as_ref() == b"author" => break,
                        Ok(Event::Eof) | Err(_) => break,
                        _ => {}
                    }
                }
                return Some(clean(&text));
            }
            Ok(Event::End(e)) if e.local_name().as_ref() == b"author" => return None,
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
        buf.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let xml = include_str!("../../tests/fixtures/arxiv_papers.xml");
        if !crate::engines::util::fixture_parses("arxiv_papers.xml") {
            return;
        }
        let results = parse_arxiv(xml, "arxiv");
        assert_eq!(results.len(), 3);
        assert!(results[0].url.starts_with("https://arxiv.org/abs/"));
        assert!(!results[0].author.is_empty());
        assert_eq!(results[0].publisher, "arXiv");
        assert!(
            results[0]
                .published
                .as_deref()
                .is_some_and(|p| !p.is_empty())
        );
    }

    #[test]
    fn parse_empty_feed_is_honest_empty() {
        let xml = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom"></feed>"#;
        assert!(parse_arxiv(xml, "arxiv").is_empty());
        assert!(parse_arxiv("not xml at all <<<", "arxiv").is_empty());
    }

    #[test]
    fn parse_decodes_xml_entities() {
        let xml = r#"<?xml version="1.0"?><feed xmlns="http://www.w3.org/2005/Atom">
<entry><title>Fish &amp; Chips</title>
<id>http://arxiv.org/abs/1234.5678v1</id>
<published>2026-01-01T00:00:00Z</published>
<summary>1 &lt; 2 &amp;&amp; 3 &gt; 2</summary>
<author><name>A &amp; B</name></author>
</entry></feed>"#;
        let results = parse_arxiv(xml, "arxiv");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Fish & Chips");
        assert_eq!(results[0].author, "A & B");
        assert!(
            results[0].description.contains("1 < 2"),
            "{:?}",
            results[0].description
        );
    }
}
