use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// Grokipedia typeahead API.
pub struct Grokipedia;

#[async_trait]
impl Engine for Grokipedia {
    fn name(&self) -> &'static str {
        "grokipedia"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    fn max_page(&self) -> u32 {
        1
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let url = parse::with_query(
            "https://grokipedia.com/api/typeahead",
            [("query", ctx.opts.query.as_str()), ("limit", "1")],
        );
        let resp = ctx.client.get(&url).await?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let json: serde_json::Value =
            serde_json::from_slice(&body).map_err(|e| Error::Parse(format!("grokipedia: {e}")))?;
        let Some(first) = json
            .get("results")
            .and_then(|r| r.as_array())
            .and_then(|a| a.first())
        else {
            return Ok(Vec::new());
        };
        let title = first
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim_matches('_')
            .to_string();
        if title.is_empty() {
            return Ok(Vec::new());
        }
        let snippet = first.get("snippet").and_then(|s| s.as_str()).unwrap_or("");
        let description = snippet
            .split_once("\n\n")
            .map(|(a, _)| a)
            .unwrap_or(snippet)
            .to_string();
        let slug = first.get("slug").and_then(|s| s.as_str()).unwrap_or("");
        Ok(vec![RawResult {
            title,
            url: format!("https://grokipedia.com/page/{slug}"),
            description,
            engine: self.name().into(),
            position: 1,
            ..Default::default()
        }])
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn parse_fixture() {
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/grokipedia.json")).unwrap();
        let first = json
            .get("results")
            .unwrap()
            .as_array()
            .unwrap()
            .first()
            .unwrap();
        let title = first
            .get("title")
            .unwrap()
            .as_str()
            .unwrap()
            .trim_matches('_');
        assert!(!title.is_empty());
    }
}
