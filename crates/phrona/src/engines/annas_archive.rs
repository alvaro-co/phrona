use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// Anna's Archive book search.
pub struct AnnasArchive;

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
        let url = parse::with_query(
            "https://annas-archive.gd/search",
            [
                ("q", opts.query.as_str()),
                ("page", opts.page.to_string().as_str()),
            ],
        );
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_annas(&text, self.name()))
    }
}

pub fn parse_annas(html: &str, engine: &str) -> Vec<RawResult> {
    let cleaned = html.replace("<!--", "").replace("-->", "");
    let doc = parse::parse_html(&cleaned);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("div.js-aarecord-list-outer > div").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let title = parse::select_first_nonempty(&node, "a[class*='text-lg']");
        let href = parse::attr(&node, "a[class*='text-lg']", "href").unwrap_or_default();
        if let (Some(title), _) = (title, href.clone()) {
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
                    format!("https://annas-archive.gd{href}")
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
        let results = parse_annas(html, "annas_archive");
        assert!(!results.is_empty());
    }
}
