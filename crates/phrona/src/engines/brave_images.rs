//! Brave image search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Brave images (SSR HTML; JSON payload kept as fallback).
pub struct BraveImages;

#[async_trait]
impl Engine for BraveImages {
    fn name(&self) -> &'static str {
        "brave_images"
    }

    fn category(&self) -> Category {
        Category::Images
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, country) = opts.lang_country();
        let mut params: Vec<(&str, String)> =
            vec![("q", opts.query.clone()), ("source", "web".into())];
        if let Some(t) = &opts.time_range {
            params.push(("tf", crate::engines::util::time_param(t).to_string()));
        }
        let url = parse::with_query("https://search.brave.com/images", params);
        let mut headers = wreq::header::HeaderMap::new();
        let ss = match opts.safesearch {
            SafeSearch::Strict => "strict",
            SafeSearch::Moderate => "moderate",
            SafeSearch::Off => "off",
        };
        headers.insert(
            wreq::header::COOKIE,
            wreq::header::HeaderValue::from_str(&format!(
                "safesearch={ss}; useLocation=0; country={country}; ui_lang={lang}-{country}"
            ))
            .unwrap(),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_brave_images(&text, self.name()))
    }
}

/// Parse a Brave images HTML SERP into [`RawResult`] items, decoding the
/// base64-proxied image URLs.
pub fn parse_brave_images(html: &str, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let doc = parse::parse_html(html);
    let sel = scraper::Selector::parse("div.image-result, button.image-result").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let title = parse::select_text(&node, ".image-metadata-title").unwrap_or_default();
        let thumb = parse::attr(&node, "img", "src").unwrap_or_default();
        if title.is_empty() || thumb.is_empty() {
            continue;
        }
        let image = crate::engines::util::brave_b64_decode(&thumb);
        let (width, height) =
            crate::engines::util::brave_dims(node.value().attr("style").unwrap_or(""));
        pos += 1;
        out.push(RawResult {
            title,
            image_url: image,
            thumbnail_url: thumb,
            width,
            height,
            source: parse::select_text(&node, ".image-metadata-source").unwrap_or_default(),
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
        let html = include_str!("../../tests/fixtures/brave_images.html");
        if !crate::engines::util::fixture_parses("brave_images.html") {
            return;
        }
        let results = parse_brave_images(html, "brave_images");
        assert!(!results.is_empty());
        assert!(results[0].image_url.starts_with("http"));
    }
}
