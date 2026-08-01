use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Google web search (HTML endpoint).
pub struct Google;

const SAFE: [&str; 3] = ["off", "medium", "high"];

impl Google {
    fn base_params(opts: &crate::options::SearchOptions) -> Vec<(&'static str, String)> {
        let mut p: Vec<(&'static str, String)> = vec![
            ("q", opts.query.clone()),
            ("filter", "0".into()),
            ("start", ((opts.page as usize - 1) * 10).to_string()),
            ("ie", "utf8".into()),
            ("oe", "utf8".into()),
        ];
        let safe = match opts.safesearch {
            SafeSearch::Off => SAFE[0],
            SafeSearch::Moderate => SAFE[1],
            SafeSearch::Strict => SAFE[2],
        };
        p.push(("safe", safe.into()));
        let (lang, country) = opts.lang_country();
        p.push(("hl", lang.clone()));
        p.push(("lr", format!("lang_{lang}")));
        if opts.region.is_some() {
            p.push(("cr", format!("country{country}")));
        }
        if let Some(t) = &opts.time_range {
            p.push(("tbs", format!("qdr:{}", util::time_param(t))));
        }
        p
    }

    pub fn search_url(opts: &crate::options::SearchOptions) -> String {
        parse::with_query("https://www.google.com/search", Self::base_params(opts))
    }
}

#[async_trait]
impl Engine for Google {
    fn name(&self) -> &'static str {
        "google"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    fn max_page(&self) -> u32 {
        50
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let url = Self::search_url(ctx.opts);
        let headers = {
            let mut h = wreq::header::HeaderMap::new();
            h.insert(
                wreq::header::COOKIE,
                wreq::header::HeaderValue::from_static("CONSENT=YES+"),
            );
            h
        };
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        let status = resp.status();
        let body = resp.bytes().await.map_err(Error::from)?;
        let text = String::from_utf8_lossy(&body);
        if status == 302
            || (status.as_u16() == 200 && text.len() < 3000 && text.contains("/sorry/"))
        {
            return Err(crate::error::Error::RateLimited(
                "google: captcha/sorry".into(),
            ));
        }
        if !status.is_success() {
            return Err(crate::error::Error::Http(format!(
                "google: status {status}"
            )));
        }
        Ok(parse_google(&text, self.name()))
    }
}

pub fn parse_google(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let container_sel = scraper::Selector::parse("div[data-hveid]").unwrap();
    let h3_sel = scraper::Selector::parse("h3").unwrap();
    let anchor_sel = scraper::Selector::parse("a[href]:has(h3)").unwrap();
    let mut seen = std::collections::HashSet::new();
    let mut pos = 0u32;

    for node in doc.select(&container_sel) {
        if node.select(&h3_sel).next().is_none() {
            continue;
        }
        let Some(anchor) = node.select(&anchor_sel).next() else {
            continue;
        };
        let title = anchor
            .select(&h3_sel)
            .next()
            .map(|h| parse::text_of(&h))
            .unwrap_or_default();
        let href = anchor.value().attr("href").unwrap_or("");
        if title.is_empty() || href.is_empty() {
            continue;
        }
        let url = parse::unwrap_google_url(href);
        if url.is_empty() || !url.starts_with("http") {
            continue;
        }
        if !seen.insert(url.clone()) {
            continue;
        }
        let description = node
            .select(&scraper::Selector::parse("div.VwiC3b").unwrap())
            .next()
            .map(|d| parse::text_of(&d))
            .filter(|t| !t.is_empty())
            .or_else(|| {
                let sel = scraper::Selector::parse("div").unwrap();
                let mut text = String::new();
                for d in node.select(&sel) {
                    let t = parse::text_of(&d);
                    if !t.is_empty() && t.len() > text.len() {
                        text = t;
                    }
                }
                if text.is_empty() { None } else { Some(text) }
            })
            .unwrap_or_default();
        pos += 1;
        out.push(RawResult {
            title,
            url,
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
        let html = include_str!("../../tests/fixtures/google_web.html");
        if crate::engines::util::is_block_page(html) {
            return;
        }
        let results = parse_google(html, "google");
        assert!(!results.is_empty());
        for r in results.iter().take(5) {
            assert!(r.url.starts_with("http"), "url: {}", r.url);
        }
    }
}
