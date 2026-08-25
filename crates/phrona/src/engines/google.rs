//! Google web search engine.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::error::{BlockDetails, Error, Result};
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Google web search (HTML endpoint).
pub struct Google;

/// Full consent cookie value; bypasses the consent.google.com wall.
pub(crate) const GOOGLE_CONSENT_COOKIE: &str = "CONSENT=YES+cb.20250101-07-p0.en+FX+419; SOCS=CAI";

/// Merge the engine's fixed cookies with operator-supplied bootstrap
/// session cookies (`engines.bootstrap_cookies`), when present.
pub(crate) fn merged_cookie(ctx: &EngineContext<'_>, engine: &str, fixed: &str) -> Option<String> {
    match ctx.shared.bootstrap_for(engine) {
        Some(b) => Some(format!("{fixed}; {b}")),
        None => Some(fixed.to_string()),
    }
}

/// Google `safe` parameter values, indexed by [`SafeSearch`]
/// (off | moderate | active). Shared with `google_images`.
pub(crate) const SAFE: [&str; 3] = ["off", "moderate", "active"];

pub(crate) fn safe_param(safesearch: SafeSearch) -> &'static str {
    match safesearch {
        SafeSearch::Off => SAFE[0],
        SafeSearch::Moderate => SAFE[1],
        SafeSearch::Strict => SAFE[2],
    }
}

impl Google {
    fn base_params(opts: &crate::options::SearchOptions) -> Vec<(&'static str, String)> {
        let mut p: Vec<(&'static str, String)> = vec![
            ("q", opts.query.clone()),
            // nfpr disables autocorrect (avoids redirect results)
            ("nfpr", "1".into()),
            ("filter", "0".into()),
            ("start", ((opts.page as usize - 1) * 10).to_string()),
            ("ie", "utf8".into()),
            ("oe", "utf8".into()),
        ];
        p.push(("safe", safe_param(opts.safesearch).into()));
        let (lang, country) = opts.lang_country();
        p.push(("hl", lang.clone()));
        p.push(("lr", format!("lang_{lang}")));
        if opts.region.is_some() {
            p.push(("cr", format!("country{country}")));
        }
        if let Some(t) = &opts.time_range {
            p.push(("tbs", format!("qdr:{}", util::time_param(t))));
        }
        p
    }

    /// Build the Google search URL for the given options.
    pub fn search_url(opts: &crate::options::SearchOptions) -> String {
        parse::with_query("https://www.google.com/search", Self::base_params(opts))
    }
}

#[async_trait]
impl Engine for Google {
    fn name(&self) -> &'static str {
        "google"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let url = Self::search_url(ctx.opts);
        let cookie = merged_cookie(ctx, self.name(), GOOGLE_CONSENT_COOKIE);
        let headers = {
            let mut h = wreq::header::HeaderMap::new();
            if let Some(c) = cookie {
                if let Ok(v) = wreq::header::HeaderValue::from_str(&c) {
                    h.insert(wreq::header::COOKIE, v);
                }
            }
            h
        };
        let resp = ctx.client.get_with_headers(&url, &headers).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let text = String::from_utf8_lossy(&body);
        let results = parse_google(&text, self.name());
        if results.is_empty() && looks_blocked(&text) {
            return Err(Error::blocked(self.name(), BlockDetails::Captcha));
        }
        Ok(results)
    }
}

/// Known Google block page that arrives with HTTP 200 text/html: the
/// "detected unusual traffic" CAPTCHA interstitial. Only two ultra-specific
/// phrases - normal SERPs legitimately embed other relay-page fragments
/// (`/sorry/index`, `enablejs`) as preload handlers, which caused false
/// blocks on healthy responses. Consulted only when parsing produced zero
/// results, so an honest empty SERP stays possible.
fn looks_blocked(text: &str) -> bool {
    if text.contains("detected unusual traffic")
        || text.contains("unusual traffic from your computer")
    {
        return true;
    }
    // The JS-relay interstitial ("click here if you are not redirected")
    // carries no result markup at all; healthy SERPs embed the same
    // fragments as preload handlers but always contain result nodes.
    let relay = text.contains("/httpservice/retry/enablejs")
        || text.contains("not redirected within a few seconds");
    relay && !text.contains("<h3")
}

/// Parse organic results (modern `[jscontroller=SC7lYd]` layout, falling back
/// to the older `div[data-hveid]` layout) plus a featured snippet, which is
/// returned as an answer marker (empty URL).
pub fn parse_google(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let mut pos = 0u32;

    if let Some(answer) = parse_featured_snippet(&doc) {
        pos += 1;
        out.push(RawResult {
            title: "Google featured snippet".into(),
            url: String::new(),
            description: answer,
            engine: engine.to_string(),
            position: pos,
            ..Default::default()
        });
    }

    let mut seen = std::collections::HashSet::new();
    for container in ["[jscontroller=SC7lYd]", "div[data-hveid]"] {
        let start = pos;
        let Ok(sel) = scraper::Selector::parse(container) else {
            continue;
        };
        for node in doc.select(&sel) {
            if node
                .select(&scraper::Selector::parse("h3").unwrap())
                .next()
                .is_none()
            {
                continue;
            }
            let Some(anchor) = node
                .select(&scraper::Selector::parse("a[href]:has(h3)").unwrap())
                .next()
            else {
                continue;
            };
            let title = parse::text_of(&anchor);
            let href = anchor.value().attr("href").unwrap_or("");
            if title.is_empty() || href.is_empty() {
                continue;
            }
            let url = parse::unwrap_google_url(href);
            if url.is_empty() || !url.starts_with("http") {
                continue;
            }
            if !seen.insert(url.clone()) {
                continue;
            }
            let description = google_description(&node);
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
        if pos > start {
            break;
        }
    }
    out
}

fn google_description(node: &scraper::ElementRef) -> String {
    for selector in [
        "div[data-sncf='2']",
        "div[data-sncf='1,2']",
        "div[style='-webkit-line-clamp:2']",
        "div.VwiC3b",
    ] {
        if let Some(d) = parse::select_text(node, selector).filter(|t| !t.is_empty()) {
            return d;
        }
    }
    let sel = scraper::Selector::parse("div").unwrap();
    let mut best = String::new();
    for d in node.select(&sel) {
        let t = parse::text_of(&d);
        if !t.is_empty() && t.len() > best.len() {
            best = t;
        }
    }
    best
}

fn parse_featured_snippet(doc: &scraper::Html) -> Option<String> {
    let sel = scraper::Selector::parse("block-component").ok()?;
    let fs = doc.select(&sel).next()?;
    if let Some(desc) = parse::select_text(
        &fs,
        "div[data-attrid='wa:/description'] > span:first-child, span[data-attrid='wa:/description']",
    )
    .filter(|t| !t.is_empty())
    {
        return Some(desc);
    }
    parse::select_text(&fs, "ul > li").filter(|t| !t.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looks_blocked_ignores_relay_fragments_in_healthy_serps() {
        // regression: healthy SERPs legitimately embed relay-page
        // fragments ("/sorry/index", "enablejs") as preload handlers -
        // they must not be classified as blocks
        let healthy = r#"<html><title>rust - Google Search</title>
            <div jscontroller="SC7lYd"><h3>Rust</h3></div>
            <script>if(location.href.includes("/sorry/index"))retry();
            fetch("/httpservice/retry/enablejs?sei=x");</script></html>"#;
        assert!(!looks_blocked(healthy));
        let blocked = r#"<html><title>Google Search</title>
            Our systems have detected unusual traffic from your computer
            network.</html>"#;
        assert!(looks_blocked(blocked));
        let relay = r#"<html><title>Google Search</title>
            Please click here if you are not redirected within a few seconds.
            <a href="/httpservice/retry/enablejs?sei=x">click here</a></html>"#;
        assert!(looks_blocked(relay), "relay must classify as blocked");
        assert!(!looks_blocked("<html><title>empty</title></html>"));
    }

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/google_web.html");
        if !crate::engines::util::fixture_parses("google_web.html") {
            return;
        }
        let results = parse_google(html, "google");
        assert!(!results.is_empty());
        for r in results.iter().take(5) {
            if !r.url.is_empty() {
                assert!(r.url.starts_with("http"), "url: {}", r.url);
            }
        }
    }
}
