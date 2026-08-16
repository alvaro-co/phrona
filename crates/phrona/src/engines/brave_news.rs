use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Brave news (HTML).
pub struct BraveNews;

#[async_trait]
impl Engine for BraveNews {
    fn name(&self) -> &'static str {
        "brave_news"
    }

    fn category(&self) -> Category {
        Category::News
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, country) = opts.lang_country();
        let mut params: Vec<(&str, String)> =
            vec![("q", opts.query.clone()), ("source", "web".into())];
        if let Some(t) = &opts.time_range {
            params.push(("tf", crate::engines::util::time_param(t).to_string()));
        }
        let url = parse::with_query("https://search.brave.com/news", params);
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::COOKIE,
            wreq::header::HeaderValue::from_str(&format!(
                "safesearch=moderate; useLocation=0; country={country}; ui_lang={lang}-{country}"
            ))
            .unwrap(),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_brave_news(&text, self.name()))
    }
}

pub fn parse_brave_news(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("div.result-wrapper").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let mut raw = match crate::engines::util::parse_brave_wrapper(&node) {
            Some(r) => r,
            None => continue,
        };
        pos += 1;
        raw.engine = engine.to_string();
        raw.position = pos;
        out.push(raw);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/brave_news.html");
        if !crate::engines::util::fixture_parses("brave_news.html") {
            return;
        }
        let results = parse_brave_news(html, "brave_news");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
