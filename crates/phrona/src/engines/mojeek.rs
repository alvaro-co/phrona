use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Mojeek web search.
pub struct Mojeek;

#[async_trait]
impl Engine for Mojeek {
    fn name(&self) -> &'static str {
        "mojeek"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, country) = opts.lang_country();
        let mut params: Vec<(&str, String)> = vec![("q", opts.query.clone())];
        if opts.safesearch != SafeSearch::Off {
            params.push(("safe", "1".into()));
        }
        if opts.page > 1 {
            params.push(("s", ((opts.page as usize - 1) * 10 + 1).to_string()));
        }
        if let Some(t) = &opts.time_range {
            let days = match t {
                crate::models::TimeRange::Day => 1,
                crate::models::TimeRange::Week => 7,
                crate::models::TimeRange::Month => 30,
                crate::models::TimeRange::Year => 365,
            };
            let since = (chrono::Utc::now().date_naive() - chrono::Days::new(days))
                .format("%Y%m%d")
                .to_string();
            params.push(("since", since));
        }
        let url = parse::with_query("https://www.mojeek.com/search", params);
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::COOKIE,
            wreq::header::HeaderValue::from_str(&format!("lb={lang}; arc={country}")).unwrap(),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_mojeek(&text, self.name()))
    }
}

pub fn parse_mojeek(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let mut pos = 0u32;
    for sel_str in ["ul.results-standard li", "ul.results li"] {
        let Ok(sel) = scraper::Selector::parse(sel_str) else {
            continue;
        };
        for node in doc.select(&sel) {
            let title = parse::select_first_nonempty(&node, "h2 a");
            let href =
                parse::attr(&node, "a.ob", "href").or_else(|| parse::attr(&node, "h2 a", "href"));
            if let (Some(title), Some(mut url)) = (title, href) {
                url = parse::unwrap_wrapper_url(&url);
                url = util::clean_url(&url);
                if !url.starts_with("http") {
                    continue;
                }
                pos += 1;
                out.push(RawResult {
                    title,
                    url,
                    description: parse::select_text(&node, "p.s").unwrap_or_default(),
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
        let html = include_str!("../../tests/fixtures/mojeek_web.html");
        if !crate::engines::util::fixture_parses("mojeek_web.html") {
            return;
        }
        let results = parse_mojeek(html, "mojeek");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
