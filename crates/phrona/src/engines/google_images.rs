//! Google image search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::{
    google::GOOGLE_CONSENT_COOKIE, google::merged_cookie, google::safe_param, util,
};
use crate::error::{BlockDetails, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// Google images (async ischj JSON).
pub struct GoogleImages;

#[async_trait]
impl Engine for GoogleImages {
    fn name(&self) -> &'static str {
        "google_images"
    }

    fn category(&self) -> Category {
        Category::Images
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, country) = opts.lang_country();
        let mut params: Vec<(&str, String)> = vec![
            ("q", opts.query.clone()),
            ("tbm", "isch".into()),
            ("hl", lang.clone()),
            ("asearch", "isch".into()),
            (
                "async",
                format!("_fmt:json,p:1,ijn:{}", opts.page.saturating_sub(1)),
            ),
            ("safe", safe_param(opts.safesearch).into()),
        ];
        if opts.region.is_some() {
            params.push(("cr", format!("country{country}")));
        }
        if let Some(t) = &opts.time_range {
            params.push(("tbs", format!("qdr:{}", util::time_param(t))));
        }
        let url = parse::with_query("https://www.google.com/search", params);
        let mut headers = wreq::header::HeaderMap::new();
        // consent cookie (+ operator bootstrap session cookies, if any).
        // keyed under "google": one operator session serves both engines.
        if let Some(c) = merged_cookie(ctx, "google", GOOGLE_CONSENT_COOKIE) {
            if let Ok(v) = wreq::header::HeaderValue::from_str(&c) {
                headers.insert(wreq::header::COOKIE, v);
            }
        }
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        // The async endpoint serves `application/json`, not HTML.
        util::check_response(self.name(), &resp, util::MediaType::Json)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        if looks_blocked(&text) {
            return Err(crate::error::Error::blocked(
                self.name(),
                BlockDetails::Captcha,
            ));
        }
        Ok(parse_google_images(&text, self.name()))
    }
}

/// Known Google block pages that arrive with HTTP 200 and a JSON-ish
/// content type (the "detected unusual traffic" interstitial). Conservative:
/// anything carrying real `ischj` data is never flagged.
fn looks_blocked(text: &str) -> bool {
    !text.contains("{\"ischj\":")
        && (text.contains("detected unusual traffic")
            || text.contains("unusual traffic from your computer"))
}

/// Parse a Google images `ischj` JSON payload (embedded in the search page
/// text) into [`RawResult`] items. A streaming deserializer reads the
/// first JSON value only, so trailing page HTML cannot fail the parse.
pub fn parse_google_images(text: &str, engine: &str) -> Vec<RawResult> {
    let Some(start) = text.find("{\"ischj\":") else {
        return Vec::new();
    };
    let mut stream =
        serde_json::Deserializer::from_str(&text[start..]).into_iter::<serde_json::Value>();
    let Some(Ok(json)) = stream.next() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let Some(meta) = json.pointer("/ischj/metadata").and_then(|m| m.as_array()) else {
        return out;
    };
    for (i, item) in meta.iter().enumerate() {
        let Some(result) = item.get("result") else {
            continue;
        };
        let url = result
            .get("referrer_url")
            .and_then(|u| u.as_str())
            .unwrap_or("");
        let title = result
            .get("page_title")
            .and_then(|t| t.as_str())
            .unwrap_or("");
        // `original_image` sits on the item level (sibling of `result`),
        // not inside it.
        let image = item
            .pointer("/original_image/url")
            .and_then(|u| u.as_str())
            .unwrap_or("");
        if url.is_empty() || image.is_empty() {
            continue;
        }
        let width = item
            .pointer("/original_image/width")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let height = item
            .pointer("/original_image/height")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        // looked up once: feeds both `source` (when no site title) and
        // `description`
        let snippet = item
            .get("text_in_grid")
            .and_then(|t| t.get("snippet"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let mut source = snippet.clone();
        if let Some(site) = result.get("site_title").and_then(|s| s.as_str()) {
            source = site.to_string();
        }
        out.push(RawResult {
            title: title.to_string(),
            url: url.to_string(),
            image_url: image.to_string(),
            thumbnail_url: item
                .get("thumbnail")
                .and_then(|t| t.get("url"))
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string(),
            width,
            height,
            source,
            description: snippet,
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
        let text = include_str!("../../tests/fixtures/google_images.json");
        if !crate::engines::util::fixture_parses("google_images.json") {
            return;
        }
        let results = parse_google_images(text, "google_images");
        assert!(!results.is_empty());
        assert!(results[0].image_url.starts_with("http"));
    }
}
