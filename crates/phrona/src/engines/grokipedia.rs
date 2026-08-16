use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
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

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let url = parse::with_query(
            "https://grokipedia.com/api/typeahead",
            [("query", ctx.opts.query.as_str()), ("limit", "1")],
        );
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Json)?;
        let body = util::read_body(resp, self.name()).await?;
        let json: serde_json::Value = util::parse_json_body(self.name(), &body)?;
        Ok(parse_grokipedia(&json, self.name()))
    }
}

/// Parse the typeahead payload into an answer marker + a page result.
pub fn parse_grokipedia(json: &serde_json::Value, engine: &str) -> Vec<RawResult> {
    let Some(first) = json
        .get("results")
        .and_then(|r| r.as_array())
        .and_then(|a| a.first())
    else {
        return Vec::new();
    };
    let title = first
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .trim_matches('_')
        .to_string();
    if title.is_empty() {
        return Vec::new();
    }
    let snippet = first.get("snippet").and_then(|s| s.as_str()).unwrap_or("");
    let description = snippet
        .split_once("\n\n")
        .map(|(a, _)| a)
        .unwrap_or(snippet)
        .to_string();
    let slug = first.get("slug").and_then(|s| s.as_str()).unwrap_or("");
    let answer = trim_answer(&description);
    vec![
        RawResult {
            title: "Grokipedia answer".into(),
            url: String::new(),
            description: answer,
            engine: engine.into(),
            position: 1,
            ..Default::default()
        },
        RawResult {
            title,
            url: format!("https://grokipedia.com/page/{slug}"),
            description,
            engine: engine.into(),
            position: 2,
            ..Default::default()
        },
    ]
}

/// The typeahead snippet is genuine answer prose; cap it at a sentence
/// boundary so `answer` fields stay concise.
fn trim_answer(s: &str) -> String {
    let max = 500;
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if c == '.' || c == '!' || c == '?' {
            end = i + 1;
            if end >= max {
                break;
            }
        }
        if i >= max {
            break;
        }
    }
    if end == 0 {
        s.chars().take(max).collect()
    } else {
        s[..end].to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        if !crate::engines::util::fixture_parses("grokipedia.json") {
            return;
        }
        let json: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/grokipedia.json")).unwrap();
        let out = parse_grokipedia(&json, "grokipedia");
        assert!(!out.is_empty(), "fixture must produce results");
        // first item is the answer marker (empty url), second the page
        assert!(out[0].url.is_empty());
        assert!(!out[1].url.is_empty());
        assert!(out[1].url.starts_with("https://grokipedia.com/page/"));
    }

    #[test]
    fn parse_handles_missing_sections() {
        assert!(parse_grokipedia(&serde_json::json!({}), "grokipedia").is_empty());
        assert!(parse_grokipedia(&serde_json::json!({"results": []}), "grokipedia").is_empty());
    }

    #[test]
    fn trim_answer_caps_at_sentence_boundary() {
        let short = "Short answer.";
        assert_eq!(trim_answer(short), short);
        // boundary inside the 500-char window: cut there
        let long = format!("{}. {}", "x".repeat(490), "y".repeat(200));
        let out = trim_answer(&long);
        assert!(out.chars().count() <= 500, "capped at 500 chars");
        assert!(out.ends_with('.'), "cut at sentence boundary");
        assert!(!out.contains('y'), "stops at the boundary");
        // no boundary within the window: hard cap at 500
        let long2 = "z".repeat(600);
        let out = trim_answer(&long2);
        assert_eq!(out.chars().count(), 500);
    }
}
