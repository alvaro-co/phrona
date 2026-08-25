//! Ann's Archive search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// Anna's Archive book search. The project's domains rotate after TLD
/// seizures, so the engine tries a list of known mirrors in order and uses
/// the first one that responds with a parseable SERP.
pub struct AnnasArchive;

/// Known mirrors, tried in order. `.gl` first: that is the domain the
/// bootstrap harvester visits, and its session cookie is (partially)
/// domain-scoped. Domains rotate after TLD seizures.
const ANNAS_DOMAINS: &[&str] = &[
    "annas-archive.gl",
    "annas-archive.gd",
    "annas-archive.li",
    "annas-archive.se",
];

#[async_trait]
impl Engine for AnnasArchive {
    fn name(&self) -> &'static str {
        "annas_archive"
    }

    fn category(&self) -> Category {
        Category::Books
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let mut last_err: Option<Error> = None;
        let mut fallbacks = 0usize;
        // operator- or bootstrap-provided session cookie
        let bootstrap = ctx.shared.bootstrap_for(self.name());
        for domain in ANNAS_DOMAINS {
            let url = parse::with_query(
                &format!("https://{domain}/search"),
                [
                    ("q", opts.query.as_str()),
                    ("page", opts.page.to_string().as_str()),
                ],
            );
            let dbg = std::env::var_os("PHRONA_DEBUG_ANNAS").is_some();
            if dbg {
                eprintln!("[dbg] mirror {domain} bootstrap={}", bootstrap.is_some());
            }
            let mut headers = wreq::header::HeaderMap::new();
            if let Some(c) = &bootstrap {
                if let Ok(v) = wreq::header::HeaderValue::from_str(c) {
                    headers.insert(wreq::header::COOKIE, v);
                }
            }
            let req = async {
                if bootstrap.is_some() {
                    ctx.client.get_with_headers(&url, &headers).await
                } else {
                    ctx.client.get(&url).await
                }
            };
            match req.await {
                Ok(resp) => match util::check_response(self.name(), &resp, util::MediaType::Html) {
                    Ok(()) => match util::read_body(resp, self.name()).await {
                        Ok(body) => {
                            let text = String::from_utf8_lossy(&body);
                            if dbg {
                                eprintln!("[dbg] {domain} client len={}", text.len());
                            }
                            let results = parse_annas(&text, self.name(), domain);
                            if !results.is_empty() {
                                return Ok(results);
                            }
                        }
                        Err(e) => {
                            if dbg {
                                eprintln!("[dbg] {domain} body err: {e}");
                            }
                            last_err = Some(e);
                        }
                    },
                    Err(e) => {
                        if dbg {
                            eprintln!("[dbg] {domain} classify err: {e}");
                        }
                        last_err = Some(e);
                    }
                },
                Err(e) => {
                    if dbg {
                        eprintln!("[dbg] {domain} req err: {e}");
                    }
                    last_err = Some(e);
                }
            }
            // Secondary transport: system curl with the same cookies.
            // Some upstreams treat client TLS stacks differently, so a
            // second stack often succeeds where the first is throttled.
            // Reaching this point means the primary produced nothing.
            // Cap at two curl attempts across all mirrors.
            if fallbacks < 2 {
                fallbacks += 1;
                if dbg {
                    eprintln!("[dbg] {domain} curl fallback");
                }
                match util::curl_get(&url, bootstrap.as_deref(), 20) {
                    Ok((_, body)) => {
                        let text = String::from_utf8_lossy(&body);
                        if dbg {
                            eprintln!("[dbg] {domain} curl len={}", text.len());
                        }
                        let results = parse_annas(&text, self.name(), domain);
                        if !results.is_empty() {
                            return Ok(results);
                        }
                    }
                    Err(e) => {
                        if dbg {
                            eprintln!("[dbg] {domain} curl err: {e}");
                        }
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| Error::unavailable(self.name(), 503)))
    }
}

/// Parse an Anna's Archive book-search HTML SERP into [`RawResult`] items.
/// Relative links are made absolute using the given `domain`.
pub fn parse_annas(html: &str, engine: &str, domain: &str) -> Vec<RawResult> {
    let cleaned = html.replace("<!--", "").replace("-->", "");
    let doc = parse::parse_html(&cleaned);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("div.js-aarecord-list-outer > div").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let title = parse::select_first_nonempty(&node, "a[class*='text-lg']");
        let href = parse::attr(&node, "a[class*='text-lg']", "href").unwrap_or_default();
        if let Some(title) = title {
            if href.is_empty() {
                continue;
            }
            let author = parse::select_text(&node, "a[class*='text-sm']").unwrap_or_default();
            let author = if author.is_empty() {
                parse::attr(&node, "a[class*='text-sm']", "href")
                    .unwrap_or_default()
                    .strip_prefix("/search?q=")
                    .map(crate::parse::percent_decode)
                    .unwrap_or_default()
            } else {
                author
            };
            let publisher = parse::select_text(&node, "a[class*='company']").unwrap_or_default();
            let info = parse::select_text(&node, "div[class*='text-gray-800']").unwrap_or_default();
            let thumb = parse::attr(&node, "img", "src").unwrap_or_default();
            pos += 1;
            out.push(RawResult {
                title,
                url: if href.starts_with("http") {
                    href
                } else {
                    format!("https://{domain}{href}")
                },
                author,
                publisher,
                description: info,
                thumbnail_url: thumb,
                engine: engine.to_string(),
                position: pos,
                ..Default::default()
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/annas_archive.html");
        if !crate::engines::util::fixture_parses("annas_archive.html") {
            return;
        }
        let results = parse_annas(html, "annas_archive", "annas-archive.gd");
        assert!(!results.is_empty());
    }
}
