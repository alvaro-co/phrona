//! Mojeek image search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Mojeek images.
pub struct MojeekImages;

#[async_trait]
impl Engine for MojeekImages {
    fn name(&self) -> &'static str {
        "mojeek_images"
    }

    fn category(&self) -> Category {
        Category::Images
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, country) = opts.lang_country();
        let mut params: Vec<(&str, String)> =
            vec![("q", opts.query.clone()), ("fmt", "images".into())];
        if opts.safesearch != SafeSearch::Off {
            params.push(("safe", "1".into()));
        }
        if opts.page > 1 {
            params.push(("s", ((opts.page as usize - 1) * 10 + 1).to_string()));
        }
        let url = parse::with_query("https://www.mojeek.com/search", params);
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::COOKIE,
            wreq::header::HeaderValue::from_str(&format!("lb={lang}; arc={country}")).unwrap(),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_mojeek_images(&text, self.name()))
    }
}

/// Parse a Mojeek images HTML SERP into [`RawResult`] items.
pub fn parse_mojeek_images(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("div#results div.image").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let url = parse::attr(&node, "a[href]", "href").unwrap_or_default();
        let thumb = parse::attr(&node, "a img", "src").unwrap_or_default();
        if url.is_empty() || thumb.is_empty() {
            continue;
        }
        let title = parse::attr(&node, "a", "data-title").unwrap_or_default();
        pos += 1;
        out.push(RawResult {
            title: if title.is_empty() { url.clone() } else { title },
            url: url.clone(),
            image_url: url,
            thumbnail_url: thumb,
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
        let html = include_str!("../../tests/fixtures/mojeek_images.html");
        if !crate::engines::util::fixture_parses("mojeek_images.html") {
            return;
        }
        let results = parse_mojeek_images(html, "mojeek_images");
        assert!(!results.is_empty());
    }
}
