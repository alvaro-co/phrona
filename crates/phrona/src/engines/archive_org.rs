//! Internet Archive search engine.
//!
//! Uses the public `advancedsearch.php` metadata API without
//! authentication or meaningful rate limits. Full-text ranking comes
//! from the Archive itself; results link at the item-details pages.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::Result;
use crate::models::{Category, RawResult};
use crate::parse;

/// Internet Archive media search (`Category::Archives`).
pub struct ArchiveOrg;

#[async_trait]
impl Engine for ArchiveOrg {
    fn name(&self) -> &'static str {
        "archive_org"
    }

    fn category(&self) -> Category {
        Category::Archives
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let rows = opts.max_results.clamp(1, 50).to_string();
        let page = opts.page.max(1).to_string();
        let url = parse::with_query(
            "https://archive.org/advancedsearch.php",
            [
                ("q", opts.query.as_str()),
                ("fl[]", "identifier"),
                ("fl[]", "title"),
                ("fl[]", "description"),
                ("fl[]", "mediatype"),
                ("fl[]", "creator"),
                ("fl[]", "date"),
                ("rows", rows.as_str()),
                ("page", page.as_str()),
                ("output", "json"),
            ],
        );
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Json)?;
        let body = util::read_body(resp, self.name()).await?;
        let json: serde_json::Value = util::parse_json_body(self.name(), &body)?;
        Ok(parse_archive_org(&json, self.name()))
    }
}

/// A metadata field that is either a string or a list of strings
/// (the API returns both shapes depending on the item).
fn first_text(v: &serde_json::Value) -> &str {
    match v {
        serde_json::Value::String(s) => s.as_str(),
        serde_json::Value::Array(a) => a.first().and_then(|x| x.as_str()).unwrap_or(""),
        _ => "",
    }
}

/// Parse an `advancedsearch.php` JSON payload into [`RawResult`] items.
/// Media type, creator and date fold into the description head; the wire
/// shape stays a plain web result (see [`crate::search::to_result_item`]).
pub fn parse_archive_org(json: &serde_json::Value, engine: &str) -> Vec<RawResult> {
    let mut out = Vec::new();
    let Some(docs) = json.pointer("/response/docs").and_then(|v| v.as_array()) else {
        return out;
    };
    let mut pos = 0u32;
    for doc in docs {
        let id = doc.get("identifier").map(first_text).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let title = doc.get("title").map(first_text).unwrap_or("");
        let description = doc.get("description").map(first_text).unwrap_or("");
        let mediatype = doc.get("mediatype").map(first_text).unwrap_or("");
        let creator = doc.get("creator").map(first_text).unwrap_or("");
        let date = doc.get("date").map(first_text).unwrap_or("");
        let mut head = String::new();
        for part in [mediatype, creator, date] {
            if part.is_empty() {
                continue;
            }
            if !head.is_empty() {
                head.push_str(" · ");
            }
            head.push_str(part);
        }
        let full = if head.is_empty() {
            description.to_string()
        } else if description.is_empty() {
            head
        } else {
            format!("{head} · {description}")
        };
        pos += 1;
        out.push(RawResult {
            title: if title.is_empty() {
                id.to_string()
            } else {
                title.to_string()
            },
            url: format!("https://archive.org/details/{id}"),
            // item descriptions are often full texts: cap the merged
            // payload so one hit cannot dominate ranking and output
            description: crate::parse::truncate(&full, 1500),
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
        let text = include_str!("../../tests/fixtures/archive_org.json");
        if !crate::engines::util::fixture_parses("archive_org.json") {
            return;
        }
        let json: serde_json::Value = serde_json::from_str(text).unwrap();
        let results = parse_archive_org(&json, "archive_org");
        assert_eq!(results.len(), 3);
        assert!(results[0].url.starts_with("https://archive.org/details/"));
    }

    #[test]
    fn parse_handles_list_fields_and_missing_titles() {
        let json: serde_json::Value = serde_json::from_str(
            r#"{"response":{"docs":[
                {"identifier":"x1","title":["T1","T1b"],"description":["D1"],"mediatype":"texts","date":"2020-01-02"},
                {"identifier":"x2"},
                {"title":"no identifier"}
            ]}}"#,
        )
        .unwrap();
        let results = parse_archive_org(&json, "archive_org");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "T1");
        assert_eq!(results[0].description, "texts · 2020-01-02 · D1");
        assert_eq!(results[1].title, "x2");
    }

    #[test]
    fn parse_empty_result_is_honest_empty() {
        let json: serde_json::Value =
            serde_json::from_str(r#"{"response":{"numFound":0,"docs":[]}}"#).unwrap();
        assert!(parse_archive_org(&json, "archive_org").is_empty());
    }
}
