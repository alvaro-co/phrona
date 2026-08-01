use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// Wikipedia opensearch API: returns the best single match with a summary.
pub struct Wikipedia;

#[async_trait]
impl Engine for Wikipedia {
    fn name(&self) -> &'static str {
        "wikipedia"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    fn max_page(&self) -> u32 {
        1
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let (lang, _) = ctx.opts.lang_country();
        let lang = lang_clean(&lang);
        let url = parse::with_query(
            &format!("https://{lang}.wikipedia.org/w/api.php"),
            [
                ("action", "opensearch"),
                ("profile", "fuzzy"),
                ("limit", "1"),
                ("search", ctx.opts.query.as_str()),
            ],
        );
        let resp = ctx.client.get(&url).await?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let json: serde_json::Value =
            serde_json::from_slice(&body).map_err(|e| Error::Parse(format!("wikipedia: {e}")))?;
        let title = json
            .get(1)
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let href = json
            .get(3)
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let (Some(title), Some(url)) = (title, href) else {
            return Ok(Vec::new());
        };
        let mut description = String::new();
        let extract_url = parse::with_query(
            &format!("https://{lang}.wikipedia.org/w/api.php"),
            [
                ("action", "query"),
                ("format", "json"),
                ("prop", "extracts"),
                ("explaintext", "1"),
                ("exintro", "1"),
                ("redirects", "1"),
                ("titles", title.as_str()),
            ],
        );
        if let Ok(resp) = ctx.client.get(&extract_url).await {
            if let Ok(bytes) = resp.bytes().await {
                if let Ok(ext) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                    if let Some(page) = ext
                        .pointer("/query/pages")
                        .and_then(|v| v.as_object())
                        .and_then(|m| m.values().next())
                    {
                        if let Some(e) = page.get("extract").and_then(|v| v.as_str()) {
                            if !e.contains("may refer to:") {
                                description = e.to_string();
                            }
                        }
                    }
                }
            }
        }
        Ok(vec![RawResult {
            title,
            url,
            description,
            engine: self.name().into(),
            position: 1,
            ..Default::default()
        }])
    }
}

fn lang_clean(lang: &str) -> String {
    if lang.is_empty() || lang == "all" || lang == "us" || lang == "gb" {
        "en".to_string()
    } else {
        lang.to_string()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn parse_fixture() {
        let json: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/wikipedia_opensearch.json"
        ))
        .unwrap();
        let title = json
            .get(1)
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        let href = json
            .get(3)
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|v| v.as_str())
            .unwrap()
            .to_string();
        assert_eq!(title, "Rust (programming language)");
        assert!(href.contains("wikipedia.org"));
    }
}
