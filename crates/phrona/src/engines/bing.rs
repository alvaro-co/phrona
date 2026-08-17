//! Bing web search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Bing web search.
pub struct Bing;

#[async_trait]
impl Engine for Bing {
    fn name(&self) -> &'static str {
        "bing"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, country) = opts.lang_country();
        let mkt = opts
            .region
            .clone()
            .unwrap_or_else(|| format!("{lang}-{country}"));
        // Bing expects a client ID (cvid) matching the SRCHHPGUSR cookie; a
        // fresh random value each query looks like a real browser and avoids
        // the no-results layout.
        let cvid = util::random_token(32);
        let mut params: Vec<(&str, String)> = vec![
            ("q", opts.query.clone()),
            ("pq", opts.query.clone()),
            ("cvid", cvid.clone()),
            ("count", "10".into()),
            ("first", ((opts.page as usize - 1) * 10).to_string()),
            ("mkt", mkt.clone()),
            ("setlang", lang.clone()),
        ];
        if let Some(t) = &opts.time_range {
            let days = match t {
                crate::models::TimeRange::Day => "d",
                crate::models::TimeRange::Week => "w",
                crate::models::TimeRange::Month => "m",
                crate::models::TimeRange::Year => "y",
            };
            let today = chrono::Utc::now().date_naive();
            let (start, end) = match days {
                "d" => (today, today),
                "w" => (today - chrono::Days::new(7), today),
                "m" => (today - chrono::Days::new(30), today),
                _ => (today - chrono::Days::new(365), today),
            };
            params.push((
                "filters",
                format!(
                    "ex1:\"ez{}_{}_{}\"",
                    5,
                    start.format("%Y%m%d"),
                    end.format("%Y%m%d")
                ),
            ));
        } else {
            // rcrse:"1" disables autocorrect (keeps the submitted query).
            params.push(("filters", "rcrse:\"1\"".into()));
        }
        let url = parse::with_query("https://www.bing.com/search", params);
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::COOKIE,
            wreq::header::HeaderValue::from_str(&format!("SRCHHPGUSR=IG={cvid}")).unwrap(),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_bing(&text, self.name()))
    }
}

/// Parse a Bing web-search HTML SERP into [`RawResult`] items, unwrapping
/// redirect and tracking URLs.
pub fn parse_bing(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("li.b_algo").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let title = parse::select_first_nonempty(&node, "h2 a");
        let href = parse::attr(&node, "h2 a", "href");
        if let (Some(title), Some(mut url)) = (title, href) {
            if url.contains("bing.com/aclick?") {
                continue;
            }
            url = parse::unwrap_bing_url(&url);
            url = parse::unwrap_wrapper_url(&url);
            url = util::clean_url(&url);
            pos += 1;
            out.push(RawResult {
                title,
                url,
                description: parse::select_text_joined(&node, "p", " "),
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
        let html = include_str!("../../tests/fixtures/bing_web.html");
        if !crate::engines::util::fixture_parses("bing_web.html") {
            return;
        }
        let results = parse_bing(html, "bing");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
