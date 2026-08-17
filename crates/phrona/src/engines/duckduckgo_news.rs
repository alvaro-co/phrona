//! DuckDuckGo news search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::engines::util::ddg_vqd;
use crate::error::Result;
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// DuckDuckGo news (JSON endpoint).
pub struct DuckDuckGoNews;

#[async_trait]
impl Engine for DuckDuckGoNews {
    fn name(&self) -> &'static str {
        "duckduckgo_news"
    }

    fn category(&self) -> Category {
        Category::News
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let vqd = ddg_vqd(ctx, &opts.query).await?;
        let p = match opts.safesearch {
            SafeSearch::Strict => "1",
            SafeSearch::Moderate => "-1",
            SafeSearch::Off => "-2",
        };
        let mut params: Vec<(&str, String)> = vec![
            ("l", opts.region_param()),
            ("o", "json".into()),
            ("noamp", "1".into()),
            ("q", opts.query.clone()),
            ("vqd", vqd),
            ("p", p.into()),
        ];
        if let Some(t) = &opts.time_range {
            params.push(("df", crate::engines::util::time_param(t).to_string()));
        }
        if opts.page > 1 {
            params.push(("s", ((opts.page as usize - 1) * 30).to_string()));
        }
        let url = parse::with_query("https://duckduckgo.com/news.js", params);
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Json)?;
        let body = util::read_body(resp, self.name()).await?;
        let json = util::parse_json_body(self.name(), &body)?;
        Ok(parse_ddg_news(&json, self.name()))
    }
}

/// Parse a DuckDuckGo news JSON response into [`RawResult`] items.
pub fn parse_ddg_news(json: &serde_json::Value, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let Some(results) = json.get("results").and_then(|r| r.as_array()) else {
        return out;
    };
    for (i, item) in results.iter().enumerate() {
        let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("");
        let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("");
        if url.is_empty() || title.is_empty() {
            continue;
        }
        let published = item
            .get("date")
            .and_then(|d| d.as_str())
            .map(|s| s.to_string());
        out.push(RawResult {
            title: title.to_string(),
            url: url.to_string(),
            description: item
                .get("excerpt")
                .and_then(|e| e.as_str())
                .unwrap_or("")
                .to_string(),
            published,
            source: item
                .get("source")
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string(),
            image_url: item
                .get("image")
                .and_then(|i| i.as_str())
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
            serde_json::from_str(include_str!("../../tests/fixtures/ddg_news.json")).unwrap();
        if !crate::engines::util::fixture_parses("ddg_news.json") {
            return;
        }
        let results = parse_ddg_news(&json, "duckduckgo_news");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
