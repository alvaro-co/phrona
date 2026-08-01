use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util::ddg_vqd;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// DuckDuckGo images (JSON endpoint).
pub struct DuckDuckGoImages;

#[async_trait]
impl Engine for DuckDuckGoImages {
    fn name(&self) -> &'static str {
        "duckduckgo_images"
    }

    fn category(&self) -> Category {
        Category::Images
    }

    fn max_page(&self) -> u32 {
        10
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let vqd = ddg_vqd(ctx, &opts.query).await?;
        let p = match opts.safesearch {
            SafeSearch::Strict | SafeSearch::Moderate => "1",
            SafeSearch::Off => "-1",
        };
        let mut f_parts: Vec<String> = Vec::new();
        if let Some(t) = &opts.time_range {
            let val = match t {
                crate::models::TimeRange::Day => "Day",
                crate::models::TimeRange::Week => "Week",
                crate::models::TimeRange::Month => "Month",
                crate::models::TimeRange::Year => "Year",
            };
            f_parts.push(format!("time:{val}"));
        }
        if let Some(extra) = &opts.filters {
            f_parts.push(extra.clone());
        }
        let mut params: Vec<(&str, String)> = vec![
            ("o", "json".into()),
            ("q", opts.query.clone()),
            ("l", opts.region_param()),
            ("vqd", vqd),
            ("p", p.into()),
            ("ct", "AT".into()),
        ];
        if !f_parts.is_empty() {
            params.push(("f", f_parts.join(",")));
        }
        if opts.page > 1 {
            params.push(("s", ((opts.page as usize - 1) * 100).to_string()));
        }
        let url = parse::with_query("https://duckduckgo.com/i.js", params);
        let resp = ctx.client.get(&url).await?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let json: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| Error::Parse(format!("duckduckgo_images: {e}")))?;
        Ok(parse_ddg_images(&json, self.name()))
    }
}

pub fn parse_ddg_images(json: &serde_json::Value, engine: &str) -> Vec<RawResult> {
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
        let width = item.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        let height = item.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        out.push(RawResult {
            title: title.to_string(),
            url: url.to_string(),
            image_url: item
                .get("image")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string(),
            thumbnail_url: item
                .get("thumbnail")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string(),
            width,
            height,
            source: item
                .get("source")
                .and_then(|s| s.as_str())
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
            serde_json::from_str(include_str!("../../tests/fixtures/ddg_images.json")).unwrap();
        let results = parse_ddg_images(&json, "duckduckgo_images");
        assert!(!results.is_empty());
        assert!(results[0].image_url.starts_with("http"));
    }

    #[test]
    fn time_params() {
        assert_eq!(
            crate::engines::util::time_param(&crate::models::TimeRange::Week),
            "w"
        );
    }
}
