use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::engines::util::ddg_vqd;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult, SafeSearch};
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
        let vqd = ddg_vqd(ctx, &opts.query).await?;
        let p = match opts.safesearch {
            SafeSearch::Strict => "1",
            SafeSearch::Moderate => "-1",
            SafeSearch::Off => "-2",
        };
        let mut f_parts: Vec<String> = Vec::new();
        if let Some(t) = &opts.time_range {
            let val = match t {
                crate::models::TimeRange::Day => "Day",
                crate::models::TimeRange::Week => "Week",
                crate::models::TimeRange::Month => "Month",
                crate::models::TimeRange::Year => "Year",
            };
            f_parts.push(format!("publishedAfter:{val}"));
        }
        if let Some(extra) = &opts.filters {
            f_parts.push(extra.clone());
        }
        let mut params: Vec<(&str, String)> = vec![
            ("l", opts.region_param()),
            ("o", "json".into()),
            ("q", opts.query.clone()),
            ("vqd", vqd),
            ("p", p.into()),
        ];
        if !f_parts.is_empty() {
            params.push(("f", f_parts.join(",")));
        }
        if opts.page > 1 {
            params.push(("s", ((opts.page as usize - 1) * 60).to_string()));
        }
        let url = parse::with_query("https://duckduckgo.com/v.js", params);
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Json)?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let json = util::parse_json_body(self.name(), &body)?;
        Ok(parse_ddg_videos(&json, self.name()))
    }
}

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
