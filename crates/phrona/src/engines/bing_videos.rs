use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::engines::util::bing_time_minutes;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Bing videos (asyncv2 endpoint).
pub struct BingVideos;

#[async_trait]
impl Engine for BingVideos {
    fn name(&self) -> &'static str {
        "bing_videos"
    }

    fn category(&self) -> Category {
        Category::Videos
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, _) = opts.lang_country();
        let mut params: Vec<(&str, String)> = vec![
            ("q", opts.query.clone()),
            ("async", "content".into()),
            ("first", ((opts.page as usize - 1) * 35 + 1).to_string()),
            ("count", "35".into()),
        ];
        let mkt = opts.region.clone().unwrap_or_else(|| format!("{lang}-US"));
        params.push(("mkt", mkt));
        if let Some(t) = &opts.time_range {
            params.push(("form", "VRFLTR".into()));
            params.push((
                "qft",
                format!(" filterui:videoage-lt{}", bing_time_minutes(t)),
            ));
        }
        let url = parse::with_query("https://www.bing.com/videos/asyncv2", params);
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_bing_videos(&text, self.name()))
    }
}

pub fn parse_bing_videos(html: &str, engine: &str) -> Vec<RawResult> {
    // Bing wraps all result markup in <noscript>, which DOM parsers treat as
    // raw text; unwrap it so the nodes become visible.
    let html = regex_strip_noscript(html);
    let doc = parse::parse_html(&html);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("div[id^=\"mc_vtvc_video\"]").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let raw = parse::attr(&node, "div.vrhdata", "vrhm")
            .or_else(|| node.value().attr("mmeta").map(str::to_string));
        let Some(raw) = raw else { continue };
        let raw = raw.replace("&quot;", "\"").replace("&amp;", "&");
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let url = json.get("murl").and_then(|v| v.as_str()).unwrap_or("");
        let title = json.get("vt").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() || title.is_empty() {
            continue;
        }
        let thumb = parse::attr(&node, "img[class^=\"rms\"]", "data-src-hq")
            .or_else(|| parse::attr(&node, "img[class^=\"rms\"]", "src"))
            .unwrap_or_default();
        pos += 1;
        out.push(RawResult {
            title: title.to_string(),
            url: url.to_string(),
            description: parse::select_text_joined(&node, ".mc_vtvc_meta_block span", " - "),
            thumbnail_url: thumb,
            duration: json
                .get("du")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            engine: engine.to_string(),
            position: pos,
            ..Default::default()
        });
    }
    out
}

fn regex_strip_noscript(html: &str) -> String {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?i)</?noscript\s*>").unwrap());
    re.replace_all(html, "").into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/bing_videos.html");
        if !crate::engines::util::fixture_parses("bing_videos.html") {
            return;
        }
        let results = parse_bing_videos(html, "bing_videos");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
