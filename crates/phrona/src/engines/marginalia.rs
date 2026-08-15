use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// Marginalia: an index of the "old internet". Only accepts short,
/// alphanumeric queries (3 words or fewer), matching the site's limits.
pub struct Marginalia;

#[async_trait]
impl Engine for Marginalia {
    fn name(&self) -> &'static str {
        "marginalia"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let query = &ctx.opts.query;
        if query.split_whitespace().count() > 3
            || !query.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ')
        {
            return Ok(Vec::new());
        }
        let url = parse::with_query(
            "https://old-search.marginalia.nu/search",
            [
                ("query", query.as_str()),
                ("profile", "default"),
                ("js", "default"),
                ("adtech", "default"),
            ],
        );
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_marginalia(&text, self.name()))
    }
}

pub fn parse_marginalia(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("section.search-result").unwrap();
    let title_sel = scraper::Selector::parse("h2 a.title, h2 a").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let title = node
            .select(&title_sel)
            .next()
            .map(|a| parse::text_of(&a))
            .unwrap_or_default();
        let href = node
            .select(&title_sel)
            .next()
            .and_then(|a| a.value().attr("href"))
            .unwrap_or("")
            .to_string();
        if title.is_empty() || !href.starts_with("http") {
            continue;
        }
        pos += 1;
        out.push(RawResult {
            title,
            url: href,
            description: parse::select_text(&node, "p.description").unwrap_or_default(),
            engine: engine.to_string(),
            position: pos,
            ..Default::default()
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/marginalia_web.html");
        if !crate::engines::util::fixture_parses("marginalia_web.html") {
            return;
        }
        let results = parse_marginalia(html, "marginalia");
        assert!(!results.is_empty());
        for r in results.iter().take(3) {
            assert!(r.url.starts_with("http"), "url: {}", r.url);
        }
    }
}
