use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult, SafeSearch};
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
            ("async", format!("_fmt:json,p:1,ijn:{}", opts.page - 1)),
            (
                "safe",
                match opts.safesearch {
                    SafeSearch::Strict => "active",
                    SafeSearch::Moderate => "moderate",
                    SafeSearch::Off => "off",
                }
                .into(),
            ),
        ];
        if opts.region.is_some() {
            params.push(("cr", format!("country{country}")));
        }
        if let Some(t) = &opts.time_range {
            params.push((
                "tbs",
                format!("qdr:{}", crate::engines::util::time_param(t)),
            ));
        }
        let url = parse::with_query("https://www.google.com/search", params);
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::COOKIE,
            wreq::header::HeaderValue::from_static("CONSENT=YES+"),
        );
        headers.insert(
            wreq::header::USER_AGENT,
            wreq::header::HeaderValue::from_static(
                "Mozilla/5.0 (Linux; Android 8.0; Pixel 2) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Mobile Safari/537.36",
            ),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_google_images(&text, self.name()))
    }
}

pub fn parse_google_images(text: &str, engine: &str) -> Vec<RawResult> {
    let Some(start) = text.find("{\"ischj\":") else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text[start..]) else {
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
        let image = result
            .get("original_image")
            .and_then(|o| o.get("url"))
            .and_then(|u| u.as_str())
            .unwrap_or("");
        if url.is_empty() || image.is_empty() {
            continue;
        }
        let width = result
            .get("original_image")
            .and_then(|o| o.get("width"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let height = result
            .get("original_image")
            .and_then(|o| o.get("height"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let mut source = item
            .get("text_in_grid")
            .and_then(|t| t.get("snippet"))
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
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
            description: item
                .get("text_in_grid")
                .and_then(|t| t.get("snippet"))
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
        let text = include_str!("../../tests/fixtures/google_images.json");
        if !crate::engines::util::fixture_parses("google_images.json") {
            return;
        }
        let results = parse_google_images(text, "google_images");
        assert!(!results.is_empty());
        assert!(results[0].image_url.starts_with("http"));
    }
}
