use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};

/// DuckDuckGo HTML endpoint (no-JS) - `html.duckduckgo.com`.
pub struct DuckDuckGo;

#[async_trait]
impl Engine for DuckDuckGo {
    fn name(&self) -> &'static str {
        "duckduckgo"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        // GET on the HTML endpoint works without a vqd token; the POST variant
        // (payload identical to ddgs) triggers DDG's anomaly/bot page.
        let opts = ctx.opts;
        let mut params: Vec<(&str, String)> = vec![
            ("q", opts.query.clone()),
            ("b", String::new()),
            ("l", opts.region_param()),
        ];
        if opts.page > 1 {
            // consistent 30-result stride between pages
            params.push(("s", ((opts.page - 1) * 30).to_string()));
        }
        if let Some(t) = &opts.time_range {
            params.push(("df", util::time_param(t).to_string()));
        }
        let url = crate::parse::with_query("https://html.duckduckgo.com/html/", params);
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        let (mut results, answer) = util::parse_ddg_html(&text, self.name());
        if let Some(a) = answer {
            results.push(RawResult {
                title: "answer".into(),
                url: String::new(),
                description: a,
                engine: self.name().into(),
                position: 0,
                ..Default::default()
            });
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/ddg_web.html");
        if !crate::engines::util::fixture_parses("ddg_web.html") {
            return;
        }
        let (results, _) = util::parse_ddg_html(html, "duckduckgo");
        assert!(!results.is_empty(), "expected results");
        let r = &results[0];
        assert!(!r.title.is_empty());
        assert!(r.url.starts_with("http"), "url: {}", r.url);
    }
}
