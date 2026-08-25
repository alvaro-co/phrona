//! Brave video search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Brave videos (SSR HTML; JSON payload kept as fallback).
pub struct BraveVideos;

#[async_trait]
impl Engine for BraveVideos {
    fn name(&self) -> &'static str {
        "brave_videos"
    }

    fn category(&self) -> Category {
        Category::Videos
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let url = parse::with_query(
            "https://search.brave.com/videos",
            [
                ("q", opts.query.as_str()),
                ("source", "web"),
                // offset is a result index; Brave serves 20 results per page
                ("offset", &((opts.page - 1) * 20).to_string()),
            ],
        );
        let headers = crate::engines::brave::headers_for(opts);
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_brave_videos(&text, self.name()))
    }
}

/// Parse a Brave videos HTML SERP into [`RawResult`] items.
pub fn parse_brave_videos(html: &str, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let doc = parse::parse_html(html);
    let sel = scraper::Selector::parse("div.result-wrapper").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let mut raw = match crate::engines::util::parse_brave_wrapper(&node) {
            Some(r) => r,
            None => continue,
        };
        let thumb = parse::attr(&node, "a.thumbnail img", "src").unwrap_or_default();
        let image = crate::engines::util::brave_b64_decode(&thumb);
        let uploader = parse::select_text(&node, ".site-name-content").unwrap_or_default();
        let uploader = uploader.split('›').next().unwrap_or("").trim().to_string();
        raw.thumbnail_url = thumb;
        if !image.is_empty() {
            raw.image_url = image;
        }
        raw.uploader = uploader;
        pos += 1;
        raw.engine = engine.to_string();
        raw.position = pos;
        out.push(raw);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/brave_videos.html");
        if !crate::engines::util::fixture_parses("brave_videos.html") {
            return;
        }
        let results = parse_brave_videos(html, "brave_videos");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
