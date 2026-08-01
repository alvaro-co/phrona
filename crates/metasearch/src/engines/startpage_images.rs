use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::startpage::fetch_sc;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

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

    fn max_page(&self) -> u32 {
        10
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let sc = fetch_sc(ctx).await?;
        let (lang, country) = opts.lang_country();
        let mut form: Vec<(&str, String)> = vec![
            ("query", opts.query.clone()),
            ("cat", "images".into()),
            ("t", "device".into()),
            ("sc", sc),
            ("language", lang.clone()),
            ("lui", lang.clone()),
            ("abp", "1".into()),
            ("abd", "0".into()),
            ("abe", "0".into()),
            ("qsr", format!("{lang}_{}", country.to_uppercase())),
            ("segment", "startpage.udog".into()),
        ];
        if opts.page > 1 {
            form.push(("page", opts.page.to_string()));
        }
        let body = parse::form_encode(form);
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::REFERER,
            wreq::header::HeaderValue::from_static("https://www.startpage.com/"),
        );
        headers.insert(
            wreq::header::ORIGIN,
            wreq::header::HeaderValue::from_static("https://www.startpage.com"),
        );
        let resp = ctx
            .client
            .post_form_with_headers("https://www.startpage.com/sp/search", &body, &headers)
            .await?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_startpage_images(&text, self.name()))
    }
}

pub fn parse_startpage_images(html: &str, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    for marker in ["React.createElement(UIStartpage.AppSerpImages, {"] {
        let Some(start) = html.find(marker) else {
            continue;
        };
        let rest = &html[start + marker.len()..];
        let Some(end) = rest.rfind("}})") else {
            continue;
        };
        let payload = format!("{{{}}}", &rest[..end + 1]);
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&payload) else {
            continue;
        };
        let Some(mainline) = json
            .pointer("/render/presenter/regions/mainline")
            .and_then(|v| v.as_array())
        else {
            continue;
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
                if url.starts_with("/av/proxy-image") {
                    if let Ok(parsed) = url::Url::parse(&format!("https://www.startpage.com{url}"))
                    {
                        let pairs: Vec<(String, String)> = parsed
                            .query_pairs()
                            .filter(|(k, _)| k == "piurl")
                            .map(|(k, v)| (k.into_owned(), v.into_owned()))
                            .collect();
                        if let Some((_, piurl)) = pairs.first() {
                            url = piurl.clone();
                        }
                    }
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
        if !out.is_empty() {
            break;
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
        let results = parse_startpage_images(html, "startpage_images");
        assert!(!results.is_empty());
        assert!(results[0].image_url.starts_with("http"));
    }
}
