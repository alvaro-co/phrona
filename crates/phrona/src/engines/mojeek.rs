//! Mojeek web search engine.
//!
//! Unverified clients receive an [ALTCHA](https://altcha.org) proof-of-work
//! page instead of results (`engines/altcha` solves it natively): solve
//! once, `POST /captcha/verify`, and the `chllg` clearance cookie lands in
//! the client's cookie jar for all subsequent requests.

use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::{altcha, util};
use crate::error::Result;
use crate::models::{Category, RawResult, SafeSearch};
use crate::parse;

/// Mojeek web search.
pub struct Mojeek;

const ORIGIN: &str = "https://www.mojeek.com";

#[async_trait]
impl Engine for Mojeek {
    fn name(&self) -> &'static str {
        "mojeek"
    }

    fn category(&self) -> Category {
        Category::Web
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let url = self.build_url(ctx)?;
        let text = self.fetch(ctx, &url).await?;
        Ok(parse_mojeek(&text, self.name()))
    }
}

impl Mojeek {
    fn build_url(&self, ctx: &EngineContext<'_>) -> Result<String> {
        let opts = ctx.opts;
        let mut params: Vec<(&str, String)> = vec![("q", opts.query.clone())];
        if opts.safesearch != SafeSearch::Off {
            params.push(("safe", "1".into()));
        }
        if opts.page > 1 {
            params.push(("s", ((opts.page as usize - 1) * 10 + 1).to_string()));
        }
        if let Some(t) = &opts.time_range {
            let days = match t {
                crate::models::TimeRange::Day => 1,
                crate::models::TimeRange::Week => 7,
                crate::models::TimeRange::Month => 30,
                crate::models::TimeRange::Year => 365,
            };
            let since = (chrono::Utc::now().date_naive() - chrono::Days::new(days))
                .format("%Y%m%d")
                .to_string();
            params.push(("since", since));
        }
        Ok(parse::with_query("https://www.mojeek.com/search", params))
    }

    /// Fetch the SERP, transparently solving the ALTCHA challenge and
    /// retrying once when the challenge page is served.
    async fn fetch(&self, ctx: &EngineContext<'_>, url: &str) -> Result<String> {
        // NOTE: no manual Cookie header here - an explicit Cookie value
        // replaces the jar for that request in wreq, which would drop the
        // `chllg` clearance cookie right after the ALTCHA dance. Locale
        // personalization (lb/arc) is sacrificed for correctness.
        let resp = ctx.client.get(url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = util::read_body(resp, self.name()).await?;
        let mut text = String::from_utf8_lossy(&body).into_owned();

        if is_challenge_page(&text) {
            solve_challenge(ctx).await?;
            let resp = ctx.client.get(url).await?;
            util::check_response(self.name(), &resp, util::MediaType::Html)?;
            let body = util::read_body(resp, self.name()).await?;
            text = String::from_utf8_lossy(&body).into_owned();
        }
        Ok(text)
    }
}

/// The challenge page carries the widget markup injected by
/// `page_specific/challenge.js`.
pub(crate) fn is_challenge_page(html: &str) -> bool {
    html.contains("captcha-wrap") || html.contains("/captcha/challenge")
}

/// Solve the ALTCHA proof-of-work and present it. On success the clearance
/// cookie is stored in the client's cookie jar.
pub(crate) async fn solve_challenge(ctx: &EngineContext<'_>) -> Result<()> {
    const ENGINE: &str = "mojeek";
    let started = std::time::Instant::now();
    let mut ch_headers = wreq::header::HeaderMap::new();
    let referer = format!("{ORIGIN}/");
    ch_headers.insert(
        wreq::header::REFERER,
        wreq::header::HeaderValue::from_str(&referer).unwrap(),
    );
    let resp = ctx
        .client
        .get_with_headers(&format!("{ORIGIN}/captcha/challenge"), &ch_headers)
        .await?;
    util::check_response(ENGINE, &resp, util::MediaType::Json)?;
    let bytes = util::read_body(resp, ENGINE).await?;
    let json: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| crate::error::Error::schema(ENGINE, "invalid ALTCHA challenge JSON"))?;
    let challenge = altcha::Challenge::parse(&json).ok_or_else(|| altcha::blocked_error(ENGINE))?;
    // CPU-bound proof-of-work: keep it off the async worker threads
    let worker = challenge.clone();
    let solved = tokio::task::spawn_blocking(move || worker.solve())
        .await
        .map_err(|_| altcha::blocked_error(ENGINE))?;
    let Some((counter, derived)) = solved else {
        return Err(altcha::blocked_error(ENGINE));
    };
    let payload =
        challenge.solution_payload(counter, &derived, started.elapsed().as_secs_f64() * 1000.0);
    let (boundary, body) = altcha::verify_body(&payload);
    let mut headers = wreq::header::HeaderMap::new();
    headers.insert(
        wreq::header::CONTENT_TYPE,
        wreq::header::HeaderValue::from_str(&format!("multipart/form-data; boundary={boundary}"))
            .unwrap(),
    );
    headers.insert(
        wreq::header::HeaderName::from_static("x-requested-with"),
        wreq::header::HeaderValue::from_static("XMLHttpRequest"),
    );
    headers.insert(
        wreq::header::REFERER,
        wreq::header::HeaderValue::from_static(ORIGIN),
    );
    headers.insert(
        wreq::header::ORIGIN,
        wreq::header::HeaderValue::from_static(ORIGIN),
    );
    let verify_url = format!("{ORIGIN}/captcha/verify");
    let resp = ctx
        .client
        .post_form_with_headers(&verify_url, &body, &headers)
        .await?;
    util::check_response(ENGINE, &resp, util::MediaType::Json)?;
    let vbytes = util::read_body(resp, ENGINE).await?;
    let verdict: serde_json::Value = serde_json::from_slice(&vbytes)
        .map_err(|_| crate::error::Error::schema(ENGINE, "invalid ALTCHA verify JSON"))?;
    if verdict.get("verified") != Some(&serde_json::Value::Bool(true)) {
        return Err(altcha::blocked_error(ENGINE));
    }
    Ok(())
}

/// Parse a Mojeek web-search HTML SERP into [`RawResult`] items.
pub fn parse_mojeek(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let mut pos = 0u32;
    for sel_str in ["ul.results-standard li", "ul.results li"] {
        let Ok(sel) = scraper::Selector::parse(sel_str) else {
            continue;
        };
        for node in doc.select(&sel) {
            let title = parse::select_first_nonempty(&node, "h2 a");
            let href =
                parse::attr(&node, "a.ob", "href").or_else(|| parse::attr(&node, "h2 a", "href"));
            if let (Some(title), Some(mut url)) = (title, href) {
                url = parse::unwrap_wrapper_url(&url);
                url = util::clean_url(&url);
                if !url.starts_with("http") {
                    continue;
                }
                pos += 1;
                out.push(RawResult {
                    title,
                    url,
                    description: parse::select_text(&node, "p.s").unwrap_or_default(),
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
        let html = include_str!("../../tests/fixtures/mojeek_web.html");
        if !crate::engines::util::fixture_parses("mojeek_web.html") {
            return;
        }
        let results = parse_mojeek(html, "mojeek");
        assert!(!results.is_empty());
        assert!(results[0].url.starts_with("http"));
    }
}
