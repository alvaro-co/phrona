use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{Error, Result};
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

    fn max_page(&self) -> u32 {
        10
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let mut form: Vec<(&str, String)> = vec![
            ("q", opts.query.clone()),
            ("b", String::new()),
            ("l", opts.region_param()),
        ];
        if opts.page > 1 {
            form.push(("s", (10 + (opts.page - 2) * 15).to_string()));
        }
        if let Some(t) = &opts.time_range {
            form.push(("df", util::time_param(t).to_string()));
        }
        let body = crate::parse::form_encode(form);
        let resp = ctx
            .client
            .post_form("https://html.duckduckgo.com/html/", &body)
            .await?;
        if !resp.status().is_success() {
            return Err(crate::error::Error::Http(format!(
                "duckduckgo: status {}",
                resp.status()
            )));
        }
        let text = resp.bytes().await.map_err(Error::from)?;
        let text = String::from_utf8_lossy(&text);
        if text.contains("challenge-form") || text.contains("anomaly") {
            return Err(crate::error::Error::RateLimited(
                "duckduckgo: captcha".into(),
            ));
        }
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
        if crate::engines::util::is_block_page(html) {
            return;
        }
        let (results, _) = util::parse_ddg_html(html, "duckduckgo");
        assert!(!results.is_empty(), "expected results");
        let r = &results[0];
        assert!(!r.title.is_empty());
        assert!(r.url.starts_with("http"), "url: {}", r.url);
    }
}
