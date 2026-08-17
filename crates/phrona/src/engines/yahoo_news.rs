//! Yahoo news search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::bing_news::normalize_date;
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Yahoo news.
pub struct YahooNews;

#[async_trait]
impl Engine for YahooNews {
    fn name(&self) -> &'static str {
        "yahoo_news"
    }

    fn category(&self) -> Category {
        Category::News
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let mut params: Vec<(&str, String)> = vec![("p", opts.query.clone())];
        if opts.page > 1 {
            params.push(("b", ((opts.page as usize - 1) * 10 + 1).to_string()));
        }
        if let Some(t) = &opts.time_range {
            params.push(("btf", crate::engines::util::time_param(t).to_string()));
        }
        let url = parse::with_query("https://news.search.yahoo.com/search", params);
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_yahoo_news(&text, self.name()))
    }
}

/// Parse a Yahoo news HTML SERP into [`RawResult`] items.
pub fn parse_yahoo_news(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let mut pos = 0u32;
    let sel = scraper::Selector::parse("li.ov-a").unwrap();
    for node in doc.select(&sel) {
        let title = parse::select_first_nonempty(&node, "h4.s-title a");
        let href = parse::attr(&node, "h4.s-title a", "href");
        if let (Some(title), Some(mut url)) = (title, href) {
            url = parse::unwrap_yahoo_url(&url);
            if !url.starts_with("http") {
                continue;
            }
            let date_raw = parse::select_text(&node, "span[class*='s-time']").unwrap_or_default();
            let date_raw = date_raw.replace("&middot;", "").trim().to_string();
            let published = normalize_date(&date_raw).or_else(|| {
                if date_raw.is_empty() {
                    None
                } else {
                    Some(date_raw.clone())
                }
            });
            let image = parse::attr(&node, "img.s-img", "src")
                .or_else(|| parse::attr(&node, "img", "data-src"))
                .or_else(|| parse::attr(&node, "img", "src"))
                .unwrap_or_default()
                .trim_start_matches("-/")
                .to_string();
            let source = parse::select_text(&node, "span[class*='s-source']")
                .or_else(|| parse::select_text(&node, "span[class*='source']"))
                .unwrap_or_default();
            pos += 1;
            out.push(RawResult {
                title,
                url,
                description: parse::select_text(&node, "p.s-desc").unwrap_or_default(),
                published,
                source,
                image_url: image,
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
        let html = include_str!("../../tests/fixtures/yahoo_news.html");
        if !crate::engines::util::fixture_parses("yahoo_news.html") {
            return;
        }
        let results = parse_yahoo_news(html, "yahoo_news");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
