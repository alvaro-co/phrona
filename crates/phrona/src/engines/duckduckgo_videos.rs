//! DuckDuckGo video search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// DuckDuckGo videos (JSON endpoint).
pub struct DuckDuckGoVideos;

#[async_trait]
impl Engine for DuckDuckGoVideos {
    fn name(&self) -> &'static str {
        "duckduckgo_videos"
    }

    fn category(&self) -> Category {
        Category::Videos
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let p = util::ddg_safesearch(opts.safesearch);
        let mut f_parts: Vec<String> = Vec::new();
        if let Some(t) = &opts.time_range {
            f_parts.push(format!("publishedAfter:{}", util::ddg_time_value(t)));
        }
        if let Some(extra) = &opts.filters {
            f_parts.push(extra.clone());
        }
        let page_offset = (opts.page as usize - 1) * 60;
        let region = opts.region_param();
        let query = opts.query.clone();
        let json = util::fetch_ddg_vertical(ctx, self.name(), move |vqd| {
            let mut params: Vec<(&str, String)> = vec![
                ("l", region.clone()),
                ("o", "json".into()),
                ("q", query.clone()),
                ("vqd", vqd.to_string()),
                ("p", p.into()),
            ];
            if !f_parts.is_empty() {
                params.push(("f", f_parts.join(",")));
            }
            if page_offset > 0 {
                params.push(("s", page_offset.to_string()));
            }
            Ok(parse::with_query("https://duckduckgo.com/v.js", params))
        })
        .await?;
        Ok(parse_ddg_videos(&json, self.name()))
    }
}

/// Parse a DuckDuckGo videos JSON response into [`RawResult`] items.
pub fn parse_ddg_videos(json: &serde_json::Value, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let Some(results) = json.get("results").and_then(|r| r.as_array()) else {
        return out;
    };
    for (i, item) in results.iter().enumerate() {
        let url = item
            .get("embed_url")
            .and_then(|u| u.as_str())
            .or_else(|| item.get("url").and_then(|u| u.as_str()))
            .unwrap_or("");
        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("");
        if url.is_empty() || title.is_empty() {
            continue;
        }
        let published = item
            .get("published")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());
        let views = item
            .get("statistics")
            .and_then(|s| s.get("viewCount"))
            .and_then(|v| v.as_str())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let thumbnail = item
            .get("images")
            .and_then(|im| im.get("large"))
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        out.push(RawResult {
            title: title.to_string(),
            url: url.to_string(),
            description: item
                .get("description")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string(),
            thumbnail_url: thumbnail,
            published,
            uploader: item
                .get("uploader")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string(),
            duration: item
                .get("duration")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string(),
            views,
            source: item
                .get("provider")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string(),
            engine: engine.to_string(),
            position: i as u32 + 1,
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
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/ddg_videos.json")).unwrap();
        if !crate::engines::util::fixture_parses("ddg_videos.json") {
            return;
        }
        let results = parse_ddg_videos(&json, "duckduckgo_videos");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
