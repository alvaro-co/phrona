use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::engines::util::bing_time_minutes;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Bing images (async endpoint with `m` JSON attributes).
pub struct BingImages;

#[async_trait]
impl Engine for BingImages {
    fn name(&self) -> &'static str {
        "bing_images"
    }

    fn category(&self) -> Category {
        Category::Images
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, _) = opts.lang_country();
        let count = opts.max_results.max(35);
        let mut params: Vec<(&str, String)> = vec![
            ("q", opts.query.clone()),
            ("async", "1".into()),
            ("first", ((opts.page as usize - 1) * count + 1).to_string()),
            ("count", count.to_string()),
        ];
        let mkt = opts.region.clone().unwrap_or_else(|| format!("{lang}-US"));
        params.push(("mkt", mkt));
        if let Some(t) = &opts.time_range {
            params.push(("qft", format!("filterui:age-lt{}", bing_time_minutes(t))));
        }
        let url = parse::with_query("https://www.bing.com/images/async", params);
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_bing_images(&text, self.name()))
    }
}

pub fn parse_bing_images(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("li:has(a.iusc)").unwrap();
    let iusc = scraper::Selector::parse("a.iusc").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let Some(a) = node.select(&iusc).next() else {
            continue;
        };
        let Some(m) = a.value().attr("m") else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(m) else {
            continue;
        };
        let title = json.get("t").and_then(|v| v.as_str()).unwrap_or("");
        let image = json.get("murl").and_then(|v| v.as_str()).unwrap_or("");
        let thumb = json.get("turl").and_then(|v| v.as_str()).unwrap_or("");
        let url = json.get("purl").and_then(|v| v.as_str()).unwrap_or("");
        if title.is_empty() || image.is_empty() {
            continue;
        }
        let (width, height) = parse_resolution(&node);
        let source = parse::select_text(&node, "div.lnkw a").unwrap_or_default();
        pos += 1;
        out.push(RawResult {
            title: title.to_string(),
            url: url.to_string(),
            image_url: image.to_string(),
            thumbnail_url: thumb.to_string(),
            width,
            height,
            source,
            engine: engine.to_string(),
            position: pos,
            ..Default::default()
        });
    }
    out
}

fn parse_resolution(node: &scraper::ElementRef) -> (u32, u32) {
    let text = parse::select_text_joined(node, ".img_info span.nowrap", " ");
    match text.split_once('x').or_else(|| text.split_once('×')) {
        Some((w, h)) => (w.trim().parse().unwrap_or(0), h.trim().parse().unwrap_or(0)),
        None => (0, 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/bing_images.html");
        if !crate::engines::util::fixture_parses("bing_images.html") {
            return;
        }
        let results = parse_bing_images(html, "bing_images");
        assert!(!results.is_empty());
        assert!(results[0].image_url.starts_with("http"));
    }
}
