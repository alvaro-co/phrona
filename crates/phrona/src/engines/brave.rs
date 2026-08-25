//! Brave web search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Brave web search.
pub struct Brave;

/// Build the cookie header shared by every Brave vertical: safesearch
/// strictness, region country and UI language are all carried as cookies
/// rather than URL parameters.
pub(crate) fn headers_for(opts: &crate::options::SearchOptions) -> wreq::header::HeaderMap {
    let (lang, country) = opts.lang_country();
    let ss = match opts.safesearch {
        SafeSearch::Strict => "strict",
        SafeSearch::Moderate => "moderate",
        SafeSearch::Off => "off",
    };
    let mut headers = wreq::header::HeaderMap::new();
    headers.insert(
        wreq::header::COOKIE,
        wreq::header::HeaderValue::from_str(&format!(
            "safesearch={ss}; useLocation=0; country={country}; ui_lang={lang}-{country}"
        ))
        .unwrap_or(wreq::header::HeaderValue::from_static(
            "safesearch=moderate",
        )),
    );
    headers
}

#[async_trait]
impl Engine for Brave {
    fn name(&self) -> &'static str {
        "brave"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let mut params: Vec<(&str, String)> = vec![
            ("q", opts.query.clone()),
            ("source", "web".into()),
            // offset is a result index; Brave serves 20 results per page
            ("offset", ((opts.page - 1) * 20).to_string()),
        ];
        if let Some(t) = &opts.time_range {
            params.push(("tf", util::time_param(t).to_string()));
        }
        let url = parse::with_query("https://search.brave.com/search", params);
        let headers = headers_for(opts);
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_brave(&text, self.name()))
    }
}

/// Parse a Brave web-search HTML SERP into [`RawResult`] items.
pub fn parse_brave(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("div[data-type=\"web\"]").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let a_sel = scraper::Selector::parse("a[href]").unwrap();
        let mut url = String::new();
        for a in node.select(&a_sel) {
            let href = a.value().attr("href").unwrap_or("");
            if href.starts_with("http") {
                url = href.to_string();
                break;
            }
        }
        if url.is_empty() {
            continue;
        }
        let title = parse::select_first_nonempty(&node, "div.title")
            .or_else(|| parse::select_first_nonempty(&node, "div.snippet-title"));
        let Some(title) = title else { continue };
        let description = parse::select_text(&node, "div.snippet-description")
            .or_else(|| parse::select_text(&node, "div.snippet-content"))
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
        let html = include_str!("../../tests/fixtures/brave_web.html");
        if !crate::engines::util::fixture_parses("brave_web.html") {
            return;
        }
        let results = parse_brave(html, "brave");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
