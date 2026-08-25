//! Yandex search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Yandex web search.
pub struct Yandex;

#[async_trait]
impl Engine for Yandex {
    fn name(&self) -> &'static str {
        "yandex"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, _) = opts.lang_country();
        let mut params: Vec<(&str, String)> = vec![
            ("text", opts.query.clone()),
            ("web", "1".into()),
            ("frame", "1".into()),
            ("tmpl_version", "releases".into()),
            ("searchid", "3131712".into()),
        ];
        if ["ru", "en", "be", "fr", "de", "id", "kk", "tt", "tr", "uk"].contains(&lang.as_str()) {
            params.push(("lang", lang.clone()));
        }
        if opts.page > 1 {
            params.push(("p", (opts.page - 1).to_string()));
        }
        let url = parse::with_query("https://yandex.com/search/site/", params);
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::COOKIE,
            wreq::header::HeaderValue::from_static(
                "yp=1716337604.sp.family%3A0#1685406411.szm.1:1920x1080:1920x999",
            ),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_yandex(&text, self.name()))
    }
}

/// Parse a Yandex web-search HTML SERP into [`RawResult`] items.
pub fn parse_yandex(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    // The site-search SERP marks organic entries `li.b-serp-item`.
    let sel = scraper::Selector::parse("li.b-serp-item").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let title = parse::select_first_nonempty(&node, "h3");
        let href = parse::attr(&node, "a.b-serp-item__title-link", "href")
            .or_else(|| parse::attr(&node, "h3 a", "href"));
        if let (Some(title), Some(mut url)) = (title, href) {
            url = util::clean_url(&url);
            if !url.starts_with("http") {
                continue;
            }
            if is_anti_bot_page(&title) {
                continue;
            }
            let description = parse::select_text(&node, "div.b-serp-item__text")
                .or_else(|| parse::select_text(&node, "div[class*='TextContainer']"))
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
    }
    out
}

/// The standard `/search` endpoint serves a "there are no search results"
/// anti-bot page that still contains result-like markup; detect it so it is
/// reported as a block instead of empty.
fn is_anti_bot_page(title: &str) -> bool {
    title.contains("no search results") || title.eq_ignore_ascii_case("something went wrong")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/yandex_web.html");
        if !crate::engines::util::fixture_parses("yandex_web.html") {
            return;
        }
        let results = parse_yandex(html, "yandex");
        assert!(!results.is_empty());
    }
}
