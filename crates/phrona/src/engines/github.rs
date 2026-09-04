//! GitHub repository search engine.
//!
//! Uses the public search API without authentication, so the shared
//! quota is tight (10 search requests/minute per egress IP). Quota
//! exhaustion (403/429) surfaces as [`Error::rate_limited`], never as a
//! block: callers back off instead of harvesting sessions. Code search
//! is intentionally out of scope - it requires authentication.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// GitHub repository search (`Category::Code`).
pub struct GitHub;

#[async_trait]
impl Engine for GitHub {
    fn name(&self) -> &'static str {
        "github"
    }

    fn category(&self) -> Category {
        Category::Code
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let per_page = opts.max_results.clamp(1, 100).to_string();
        let page = opts.page.max(1).to_string();
        let url = parse::with_query(
            "https://api.github.com/search/repositories",
            [
                ("q", opts.query.as_str()),
                ("per_page", per_page.as_str()),
                ("page", page.as_str()),
            ],
        );
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::ACCEPT,
            wreq::header::HeaderValue::from_static("application/vnd.github+json"),
        );
        // pin the REST version so the payload shape cannot drift silently
        headers.insert(
            wreq::header::HeaderName::from_static("x-github-api-version"),
            wreq::header::HeaderValue::from_static("2022-11-28"),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        // search quota exhaustion arrives as 403 (or 429): a slowdown
        // signal, not an anti-bot block
        let status = resp.status().as_u16();
        if status == 403 || status == 429 {
            return Err(Error::rate_limited(self.name(), retry_after(&resp)));
        }
        util::check_response(self.name(), &resp, util::MediaType::Json)?;
        let body = util::read_body(resp, self.name()).await?;
        let json: serde_json::Value = util::parse_json_body(self.name(), &body)?;
        Ok(parse_github(&json, self.name()))
    }
}

/// Backoff hint for a quota response: the `Retry-After` header when
/// present, else the delta to `x-ratelimit-reset` (unix epoch seconds),
/// else none.
fn retry_after(resp: &wreq::Response) -> Option<std::time::Duration> {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let headers = resp.headers();
    if let Some(secs) = headers
        .get(wreq::header::RETRY_AFTER)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        return Some(Duration::from_secs(secs));
    }
    let reset: u64 = headers
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Duration::from_secs(reset.saturating_sub(now)))
}

/// Parse a GitHub repository-search JSON payload into [`RawResult`] items.
/// Stars and language fold into the description; the wire shape stays a
/// plain web result (see [`crate::search::to_result_item`]).
pub fn parse_github(json: &serde_json::Value, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let Some(items) = json.get("items").and_then(|v| v.as_array()) else {
        return out;
    };
    let mut pos = 0u32;
    for item in items {
        let name = item.get("full_name").and_then(|v| v.as_str()).unwrap_or("");
        let url = item.get("html_url").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() || url.is_empty() {
            continue;
        }
        let stars = item
            .get("stargazers_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let language = item.get("language").and_then(|v| v.as_str()).unwrap_or("");
        let about = item
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let mut meta = format!("★ {stars}");
        if !language.is_empty() {
            meta.push_str(&format!(" · {language}"));
        }
        let description = if about.is_empty() {
            meta
        } else {
            format!("{meta} · {about}")
        };
        pos += 1;
        out.push(RawResult {
            title: name.to_string(),
            url: url.to_string(),
            description,
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
        let text = include_str!("../../tests/fixtures/github_code.json");
        if !crate::engines::util::fixture_parses("github_code.json") {
            return;
        }
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let results = parse_github(&json, "github");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("https://github.com/"));
        assert!(results[0].description.contains('★'));
    }

    #[test]
    fn parse_empty_result_is_honest_empty() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"total_count":0,"items":[]}"#).unwrap();
        assert!(parse_github(&json, "github").is_empty());
    }
}
