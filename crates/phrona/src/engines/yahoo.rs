use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// Yahoo web search.
pub struct Yahoo;

#[async_trait]
impl Engine for Yahoo {
    fn name(&self) -> &'static str {
        "yahoo"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, _) = opts.lang_country();
        let mut params: Vec<(&str, String)> = vec![("p", opts.query.clone())];
        if opts.page > 1 {
            // default 10 results per page, next page starts at 11
            params.push(("b", ((opts.page as usize - 1) * 10 + 1).to_string()));
        } else {
            params.push(("iscqry", String::new()));
        }
        if let Some(t) = &opts.time_range {
            params.push(("btf", util::time_param(t).to_string()));
        }
        let token = util::random_token(18);
        let token2 = util::random_token(35);
        let url = parse::with_query(
            &format!("https://search.yahoo.com/search;_ylt={token};_ylu={token2}"),
            params,
        );
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::COOKIE,
            wreq::header::HeaderValue::from_str(&format!(
                "sB=v=1&vm=p&fl=1&vl=lang_{lang}&pn=10&rw=new&userset=1"
            ))
            .unwrap(),
        );
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_yahoo(&text, self.name()))
    }
}

pub fn parse_yahoo(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let mut pos = 0u32;
    for sel_str in ["div.algo-sr", "div.relsrch"] {
        let Ok(sel) = scraper::Selector::parse(sel_str) else {
            continue;
        };
        for node in doc.select(&sel) {
            let title = parse::select_first_nonempty(&node, ".compTitle h3")
                .or_else(|| parse::select_first_nonempty(&node, "h3"));
            let href = parse::attr(&node, ".compTitle h3 a", "href")
                .or_else(|| parse::attr(&node, ".compTitle a", "href"))
                .or_else(|| parse::attr(&node, "h3 a", "href"));
            if let (Some(title), Some(mut url)) = (title, href) {
                if url.contains("aclick") {
                    continue;
                }
                url = parse::unwrap_yahoo_url(&url);
                url = parse::unwrap_wrapper_url(&url);
                url = util::clean_url(&url);
                if !url.starts_with("http") {
                    continue;
                }
                pos += 1;
                out.push(RawResult {
                    title,
                    url,
                    description: parse::select_text(&node, ".compText").unwrap_or_default(),
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
        let html = include_str!("../../tests/fixtures/yahoo_web.html");
        if !crate::engines::util::fixture_parses("yahoo_web.html") {
            return;
        }
        let results = parse_yahoo(html, "yahoo");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
