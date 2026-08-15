use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
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
        util::check_response(self.name(), &resp, util::MediaType::Json)?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let json: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|_| Error::schema("wikipedia", "invalid JSON response"))?;
        let Some((title, url)) = parse_opensearch(&json) else {
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

/// Extract (title, url) from an opensearch response.
pub fn parse_opensearch(json: &serde_json::Value) -> Option<(String, String)> {
    let title = json
        .get(1)
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let url = json
        .get(3)
        .and_then(|v| v.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Some((title?, url?))
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
    use super::*;

    #[test]
    fn parse_fixture() {
        if !crate::engines::util::fixture_parses("wikipedia_opensearch.json") {
            return;
        }
        let json: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/wikipedia_opensearch.json"
        ))
        .unwrap();
        let (title, url) = parse_opensearch(&json).expect("fixture must parse");
        assert_eq!(title, "Rust (programming language)");
        assert!(url.contains("wikipedia.org"));
    }

    #[test]
    fn parse_handles_missing_sections() {
        assert!(parse_opensearch(&serde_json::json!({})).is_none());
        assert!(parse_opensearch(&serde_json::json!([null, [], [], []])).is_none());
    }

    #[test]
    fn lang_clean_falls_back_to_english() {
        assert_eq!(lang_clean(""), "en");
        assert_eq!(lang_clean("all"), "en");
        assert_eq!(lang_clean("us"), "en");
        assert_eq!(lang_clean("gb"), "en");
        assert_eq!(lang_clean("de"), "de");
    }
}
