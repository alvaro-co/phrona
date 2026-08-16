use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{BlockDetails, Error, Result};
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Startpage web search (POST with anti-bot `sc` token).
pub struct Startpage;

#[async_trait]
impl Engine for Startpage {
    fn name(&self) -> &'static str {
        "startpage"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let sc = fetch_sc(ctx).await?;
        let (lang, country) = opts.lang_country();
        let qadf = match opts.safesearch {
            SafeSearch::Strict => "heavy",
            SafeSearch::Moderate => "moderate",
            SafeSearch::Off => "none",
        };
        let mut form: Vec<(&str, String)> = vec![
            ("query", opts.query.clone()),
            ("cat", "web".into()),
            ("t", "device".into()),
            ("sc", sc),
            ("language", lang.clone()),
            ("lui", lang.clone()),
            ("abp", "1".into()),
            ("abd", "0".into()),
            ("abe", "0".into()),
            ("qsr", format!("{lang}_{}", country.to_uppercase())),
            ("qadf", qadf.into()),
            ("segment", "organic".into()),
        ];
        if opts.page > 1 {
            form.push(("page", opts.page.to_string()));
        }
        if let Some(t) = &opts.time_range {
            form.push((
                "with_date",
                parse::collapse(&format!("{t:?}")).to_lowercase(),
            ));
        }
        let body = parse::form_encode(form);
        let mut headers = wreq::header::HeaderMap::new();
        headers.insert(
            wreq::header::REFERER,
            wreq::header::HeaderValue::from_static("https://www.startpage.com/"),
        );
        headers.insert(
            wreq::header::ORIGIN,
            wreq::header::HeaderValue::from_static("https://www.startpage.com"),
        );
        let resp = ctx
            .client
            .post_form_with_headers("https://www.startpage.com/sp/search", &body, &headers)
            .await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_startpage(&text, self.name()))
    }
}

pub async fn fetch_sc(ctx: &EngineContext<'_>) -> Result<String> {
    if let Some(sc) = ctx.shared.sc_get() {
        return Ok(sc);
    }
    let resp = ctx.client.get("https://www.startpage.com/").await?;
    let body = util::read_body(resp, "startpage").await?;
    let text = String::from_utf8_lossy(&body);
    let sc = {
        let doc = parse::parse_html(&text);
        parse::doc_attr(&doc, "form#search input[name=\"sc\"]", "value")
            .or_else(|| parse::doc_attr(&doc, "input[name=\"sc\"]", "value"))
            .ok_or_else(|| Error::blocked("startpage", BlockDetails::BotDetection))?
    };
    ctx.shared.sc_set(sc.clone());
    Ok(sc)
}

/// Parse startpage results: embedded JSON first, HTML fallback.
pub fn parse_startpage(html: &str, engine: &str) -> Vec<RawResult> {
    if let Some(results) = parse_startpage_json(html, engine) {
        if !results.is_empty() {
            return results;
        }
    }
    parse_startpage_html(html, engine)
}

fn parse_startpage_json(html: &str, engine: &str) -> Option<Vec<RawResult>> {
    for marker in [
        "React.createElement(UIStartpage.AppSerpWeb, {",
        "React.createElement(UIStartpage.AppSerp, {",
    ] {
        let start = html.find(marker)?;
        let rest = &html[start + marker.len()..];
        let end = rest.rfind("}}")?;
        let payload = format!("{{{}}}", &rest[..end]);
        let json: serde_json::Value = serde_json::from_str(&payload).ok()?;
        let mut out = Vec::new();
        let mut pos = 0u32;
        if let Some(regions) = json
            .pointer("/render/presenter/regions/mainline")
            .and_then(|v| v.as_array())
        {
            for section in regions {
                let Some(results) = section.get("results").and_then(|r| r.as_array()) else {
                    continue;
                };
                for item in results {
                    let click_url = item.get("clickUrl").and_then(|u| u.as_str()).unwrap_or("");
                    let title = item.get("title").and_then(|t| t.as_str()).unwrap_or("");
                    if click_url.is_empty() || title.is_empty() {
                        continue;
                    }
                    let description = item
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("");
                    pos += 1;
                    out.push(RawResult {
                        title: title.to_string(),
                        url: click_url.to_string(),
                        description: description.to_string(),
                        engine: engine.to_string(),
                        position: pos,
                        ..Default::default()
                    });
                }
            }
        }
        if !out.is_empty() {
            return Some(out);
        }
    }
    None
}

fn parse_startpage_html(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let mut pos = 0u32;
    for sel_str in [
        "div.result[class*=\"w-gl__result\"]",
        "div.w-gl__result",
        "div.result",
    ] {
        let Ok(sel) = scraper::Selector::parse(sel_str) else {
            continue;
        };
        for node in doc.select(&sel) {
            let title = parse::select_first_nonempty(&node, "h2")
                .or_else(|| parse::select_first_nonempty(&node, "h3"));
            let href = parse::attr(&node, "a[href]", "href");
            if let (Some(title), Some(mut url)) = (title, href) {
                url = parse::unwrap_wrapper_url(&url);
                if !url.starts_with("http") {
                    continue;
                }
                let description = parse::select_text(&node, "p.w-gl__description")
                    .or_else(|| parse::select_text(&node, ".w-gl__description"))
                    .or_else(|| parse::select_text(&node, "p"))
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
        let html = include_str!("../../tests/fixtures/startpage_web.html");
        if !crate::engines::util::fixture_parses("startpage_web.html") {
            return;
        }
        let results = parse_startpage(html, "startpage");
        assert!(!results.is_empty(), "expected results");
    }
}
