//! Startpage image search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::startpage::{build_form, fetch_sc, post_search};
use crate::error::Result;
use crate::models::{Category, RawResult};

/// Startpage images.
pub struct StartpageImages;

#[async_trait]
impl Engine for StartpageImages {
    fn name(&self) -> &'static str {
        "startpage_images"
    }

    fn category(&self) -> Category {
        Category::Images
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let sc = fetch_sc(ctx).await?;
        let form = build_form(opts, "images", sc);
        let text = post_search(ctx, self.name(), &form).await?;
        Ok(parse_startpage_images(&text, self.name()))
    }
}

/// Parse a Startpage images HTML SERP into [`RawResult`] items, unwrapping
/// the React/JSON payload embedded in the page.
pub fn parse_startpage_images(html: &str, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let marker = "React.createElement(UIStartpage.AppSerpImages, {";
    let Some(start) = html.find(marker) else {
        return out;
    };
    let rest = &html[start + marker.len()..];
    let Some(end) = rest.rfind("}})") else {
        return out;
    };
    let payload = format!("{{{}}}", &rest[..end + 1]);
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload) else {
        return out;
    };
    let Some(mainline) = json
        .pointer("/render/presenter/regions/mainline")
        .and_then(|v| v.as_array())
    else {
        return out;
    };
    let mut pos = 0u32;
    for section in mainline {
        let Some(results) = section.get("results").and_then(|r| r.as_array()) else {
            continue;
        };
        for item in results {
            let mut url = item
                .get("clickUrl")
                .and_then(|u| u.as_str())
                .or_else(|| item.get("altClickUrl").and_then(|u| u.as_str()))
                .unwrap_or("")
                .to_string();
            if url.starts_with("/av/proxy-image")
                && let Ok(parsed) = url::Url::parse(&format!("https://www.startpage.com{url}"))
                && let Some(piurl) = parsed
                    .query_pairs()
                    .find(|(k, _)| k == "piurl")
                    .map(|(_, v)| v.into_owned())
            {
                url = piurl;
            }
            let image = item
                .get("rawImageUrl")
                .and_then(|u| u.as_str())
                .unwrap_or("");
            if url.is_empty() || image.is_empty() {
                continue;
            }
            pos += 1;
            out.push(RawResult {
                title: item
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: url.to_string(),
                image_url: image.to_string(),
                thumbnail_url: item
                    .get("thumbnailUrl")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                width: item.get("width").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                height: item.get("height").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                description: item
                    .get("description")
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
        let html = include_str!("../../tests/fixtures/startpage_images.html");
        if !crate::engines::util::fixture_parses("startpage_images.html") {
            return;
        }
        let results = parse_startpage_images(html, "startpage_images");
        assert!(!results.is_empty());
        assert!(results[0].image_url.starts_with("http"));
    }
}
