use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{BlockDetails, Error, Result};
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Qwant web search (JSON API v3).
pub struct Qwant;

#[async_trait]
impl Engine for Qwant {
    fn name(&self) -> &'static str {
        "qwant"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, country) = opts.lang_country();
        let locale = format!("{lang}_{}", country.to_uppercase());
        let ss = match opts.safesearch {
            SafeSearch::Strict => "2",
            SafeSearch::Moderate => "1",
            SafeSearch::Off => "0",
        };
        let tgp = util_random_tgp();
        let url = parse::with_query(
            "https://api.qwant.com/v3/search/web",
            [
                ("q", opts.query.as_str()),
                ("count", "10"),
                ("locale", locale.as_str()),
                ("offset", &((opts.page as usize - 1) * 10).to_string()),
                ("safesearch", ss),
                ("device", "desktop"),
                ("displayed", "true"),
                ("llm", "true"),
                ("tgp", tgp.as_str()),
            ],
        );
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::ACCEPT,
            wreq::header::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            wreq::header::REFERER,
            wreq::header::HeaderValue::from_static("https://www.qwant.com/"),
        );
        headers.insert(
            wreq::header::ORIGIN,
            wreq::header::HeaderValue::from_static("https://www.qwant.com"),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Json)?;
        let body = util::read_body(resp, self.name()).await?;
        let json: serde_json::Value = crate::engines::util::parse_json_body(self.name(), &body)?;
        if json.get("status").and_then(|s| s.as_str()) != Some("success") {
            let err = json.get("error_code").and_then(|e| e.as_i64()).unwrap_or(0);
            if err == 24 {
                return Err(Error::rate_limited(self.name(), None));
            }
            if json.get("url").is_some() {
                return Err(Error::blocked(self.name(), BlockDetails::Captcha));
            }
            return Ok(Vec::new());
        }
        Ok(parse_qwant(&json, self.name()))
    }
}

fn util_random_tgp() -> String {
    use rand::Rng;
    (rand::rng().random_range(1..=3)).to_string()
}

pub fn parse_qwant(json: &serde_json::Value, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let mut pos = 0u32;
    let Some(mainline) = json
        .pointer("/data/result/items/mainline")
        .and_then(|v| v.as_array())
    else {
        return out;
    };
    for section in mainline {
        let Some(items) = section.get("items").and_then(|v| v.as_array()) else {
            continue;
        };
        for item in items {
            let url = item.get("url").and_then(|u| u.as_str()).unwrap_or("");
            let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("");
            if url.is_empty() || title.is_empty() {
                continue;
            }
            pos += 1;
            out.push(RawResult {
                title: title.to_string(),
                url: url.to_string(),
                description: item
                    .get("desc")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string(),
                engine: engine.to_string(),
                position: pos,
                ..Default::default()
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let text = include_str!("../../tests/fixtures/qwant_web.json");
        if !crate::engines::util::fixture_parses("qwant_web.json") {
            return;
        }
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let results = parse_qwant(&json, "qwant");
        assert!(!results.is_empty());
    }
}
