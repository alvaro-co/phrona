//! Startpage web search engine.
//!
//! Startpage fronts its SERP with an [Anubis](https://anubis.techaro.lol)
//! proof-of-work interstitial; [`super::anubis`] solves it natively so the
//! POST search flow stays a plain two-step (`sc` token -> results).

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::{anubis, util};
use crate::error::{BlockDetails, Error, Result};
use crate::models::{Category, RawResult, SafeSearch, TimeRange};
use crate::parse;

/// Startpage web search (POST with anti-bot `sc` token).
pub struct Startpage;

const ORIGIN: &str = "https://www.startpage.com";

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
        let form = build_form(opts, "web", sc);
        let text = post_search(ctx, self.name(), &form).await?;
        Ok(parse_startpage(&text, self.name()))
    }
}

/// Build the shared POST body for the web and images variants. Keys are
/// all static literals, so they stay borrowed instead of pointlessly
/// allocated per search.
pub(crate) fn build_form(
    opts: &crate::options::SearchOptions,
    cat: &'static str,
    sc: String,
) -> Vec<(&'static str, String)> {
    let (lang, country) = opts.lang_country();
    let qadf = match opts.safesearch {
        SafeSearch::Strict => "heavy",
        SafeSearch::Moderate => "moderate",
        SafeSearch::Off => "none",
    };
    let mut form: Vec<(&'static str, String)> = vec![
        ("query", opts.query.clone()),
        ("cat", cat.into()),
        ("t", "device".into()),
        ("sc", sc),
        ("language", lang.clone()),
        ("lui", lang.clone()),
        ("abp", "1".into()),
        ("abd", "0".into()),
        ("abe", "0".into()),
        ("qsr", format!("{lang}_{}", country.to_uppercase())),
        ("qadf", qadf.into()),
    ];
    if cat == "images" {
        // the images vertical is only served on this segment
        form.push(("segment", "startpage.udog".into()));
    } else {
        form.push(("segment", "organic".into()));
    }
    if opts.page > 1 {
        form.push(("page", opts.page.to_string()));
    }
    if let Some(t) = &opts.time_range {
        form.push(("with_date", with_date_value(t).into()));
    }
    form
}

/// The explicit `with_date` form values accepted by Startpage
/// (previously derived from the `Debug` string of [`TimeRange`], which was
/// fragile against enum renames).
fn with_date_value(t: &TimeRange) -> &'static str {
    match t {
        TimeRange::Day => "day",
        TimeRange::Week => "week",
        TimeRange::Month => "month",
        TimeRange::Year => "year",
    }
}

/// Issue the search POST, transparently solving an Anubis interstitial and
/// retrying once when one is served. Returns the final response body.
pub(crate) async fn post_search(
    ctx: &EngineContext<'_>,
    engine: &'static str,
    form: &[(&'static str, String)],
) -> Result<String> {
    let url = format!("{ORIGIN}/sp/search");
    let mut headers = wreq::header::HeaderMap::new();
    headers.insert(
        wreq::header::REFERER,
        wreq::header::HeaderValue::from_static(ORIGIN),
    );
    headers.insert(
        wreq::header::ORIGIN,
        wreq::header::HeaderValue::from_static(ORIGIN),
    );
    let body = parse::form_encode(form.iter().map(|(k, v)| (*k, v.as_str())));
    let resp = ctx
        .client
        .post_form_with_headers(&url, &body, &headers)
        .await?;
    util::check_response(engine, &resp, util::MediaType::Html)?;
    let bytes = util::read_body(resp, engine).await?;
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if let Some(challenge) = anubis::Challenge::present_in(&text)
        .then(|| anubis::Challenge::extract(&text))
        .flatten()
    {
        challenge.redeem(ctx.client, ORIGIN, &url).await?;
        let resp = ctx
            .client
            .post_form_with_headers(&url, &body, &headers)
            .await?;
        util::check_response(engine, &resp, util::MediaType::Html)?;
        let bytes = util::read_body(resp, engine).await?;
        text = String::from_utf8_lossy(&bytes).into_owned();
    }
    if looks_blocked(&text) {
        return Err(Error::blocked(engine, BlockDetails::Captcha));
    }
    Ok(text)
}

/// Known Startpage block/anomaly pages that arrive with HTTP 200.
/// Conservative markers only - an honest empty result must stay possible.
fn looks_blocked(text: &str) -> bool {
    const MARKERS: [&str; 3] = [
        "enable JavaScript to continue", // generic interstitial phrasing
        "blocked because",               // anomaly notice
        "no-access-page",
    ];
    MARKERS.iter().any(|m| text.contains(m)) && !text.contains("AppSerp")
}

/// Fetch the Startpage anti-bot `sc` token from the homepage, caching it in
/// the shared context (used by the multi-step `sc -> search` flow).
pub async fn fetch_sc(ctx: &EngineContext<'_>) -> Result<String> {
    if let Some(sc) = ctx.shared.sc_get() {
        return Ok(sc);
    }
    let resp = ctx.client.get(ORIGIN).await?;
    util::check_response("startpage", &resp, util::MediaType::Html)?;
    let body = util::read_body(resp, "startpage").await?;
    let mut text = String::from_utf8_lossy(&body).into_owned();
    if anubis::Challenge::present_in(&text) {
        if let Some(challenge) = anubis::Challenge::extract(&text) {
            challenge.redeem(ctx.client, ORIGIN, ORIGIN).await?;
            let resp = ctx.client.get(ORIGIN).await?;
            let body = util::read_body(resp, "startpage").await?;
            text = String::from_utf8_lossy(&body).into_owned();
        }
    }
    let doc = parse::parse_html(&text);
    let sc = parse::doc_attr(&doc, "form#search input[name=\"sc\"]", "value")
        .or_else(|| parse::doc_attr(&doc, "input[name=\"sc\"]", "value"))
        .ok_or_else(|| Error::blocked("startpage", BlockDetails::BotDetection))?;
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
