//! Shared engine helpers (HTTP checks, token parsing, decoding).

use std::time::Duration;

use crate::engine::EngineContext;
use crate::error::{BlockDetails, Error, Result};
use crate::models::{RawResult, TimeRange};
use crate::parse;

/// The media type an engine expects from its endpoint (RFC 9110 content
/// negotiation). Every engine declares one so responses can be classified
/// from HTTP metadata alone.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Expects an HTML response.
    Html,
    /// Expects a JSON response.
    Json,
    /// No content-type expectation (endpoints with heterogeneous types).
    Any,
}

/// Uniform HTTP-semantics check applied to every engine response.
///
/// Classification uses only HTTP metadata — status code, anti-bot response
/// headers and `Content-Type` — never body phrasing:
///
/// * `cf-mitigated` / `cf-challenge` / `cf-ray`+403 →
///   [`crate::error::ErrorKind::Blocked`] (Cloudflare); `x-datadome` →
///   captcha
/// * `429` → [`crate::error::ErrorKind::RateLimited`] (honoring `Retry-After`)
/// * `403` → [`crate::error::ErrorKind::Blocked`]
/// * other non-2xx → [`crate::error::ErrorKind::UpstreamUnavailable`]
/// * a 2xx response whose `Content-Type` contradicts the expected
///   [`MediaType`] → [`crate::error::ErrorKind::MalformedPayload`] (a
///   structural deviation, not a rate limit)
///
/// A 2xx response of the expected type is trusted as-is; whether it actually
/// contains results is the parser's job (an empty parse is an honest "no
/// results", not a hidden failure).
pub fn check_response(
    engine: &'static str,
    resp: &wreq::Response,
    expect: MediaType,
) -> Result<()> {
    let content_type = resp
        .headers()
        .get(wreq::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    classify(
        engine,
        resp.status(),
        resp.headers(),
        content_type.as_deref(),
        expect,
    )
}

/// Pure classifier behind [`check_response`] (kept separate so the HTTP
/// semantics are directly unit-testable).
pub fn classify(
    engine: &'static str,
    status: wreq::StatusCode,
    headers: &wreq::header::HeaderMap,
    content_type: Option<&str>,
    expect: MediaType,
) -> Result<()> {
    let status_u16 = status.as_u16();

    // 1. Header-level anti-bot signals (standard and cheap).
    if headers.contains_key("cf-mitigated")
        || headers.contains_key("cf-challenge")
        || headers.contains_key("x-datadome")
        || (status == wreq::StatusCode::FORBIDDEN && headers.contains_key("cf-ray"))
    {
        let details = if headers.contains_key("x-datadome") {
            BlockDetails::Captcha
        } else {
            BlockDetails::Cloudflare
        };
        return Err(Error::blocked(engine, details));
    }

    // 2. HTTP status codes.
    match status_u16 {
        429 => {
            let retry_after = headers
                .get(wreq::header::RETRY_AFTER)
                .and_then(|h| h.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .map(Duration::from_secs);
            return Err(Error::rate_limited(engine, retry_after));
        }
        403 => return Err(Error::blocked(engine, BlockDetails::BotDetection)),
        500..=599 => return Err(Error::unavailable(engine, status_u16)),
        _ if !status.is_success() => return Err(Error::unavailable(engine, status_u16)),
        _ => {}
    }

    // 3. Content-Type contract. A JSON endpoint served a document type, or
    // an HTML endpoint served something else, is a structural deviation.
    if let Some(ct) =
        content_type.map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase())
    {
        let ok = match expect {
            MediaType::Html => ct == "text/html" || ct == "application/xhtml+xml",
            // Other types (`text/plain`, `text/javascript`, ...) are left to
            // `parse_json_body`, which validates the body grammar itself.
            MediaType::Json => !matches!(
                ct.as_str(),
                "text/html" | "application/xhtml+xml" | "text/xml" | "application/xml"
            ),
            MediaType::Any => true,
        };
        if !ok {
            return Err(Error::schema(engine, "unexpected content-type"));
        }
    }
    Ok(())
}

/// Hard cap for a single engine response body. SERP pages are a few hundred
/// KiB at most; anything past this limit is a hostile or misconfigured
/// upstream, and materializing it would exhaust server memory.
const MAX_RESPONSE_BODY: usize = 2 * 1024 * 1024;

const BODY_TOO_LARGE: &str = "response body exceeds 2 MiB size limit";

/// Read the full response body with a hard size cap (and an early
/// `Content-Length` check), so a rogue upstream cannot OOM the server via
/// an infinite or multi-gigabyte stream. Consumes the response; call
/// [`check_response`] first if status/headers still need validating.
pub async fn read_body(resp: wreq::Response, engine: &'static str) -> Result<Vec<u8>> {
    use futures::StreamExt;
    if resp
        .content_length()
        .is_some_and(|len| len as usize > MAX_RESPONSE_BODY)
    {
        return Err(Error::schema(engine, BODY_TOO_LARGE));
    }
    // reserve up-front when the upstream declares its size: avoids repeated
    // reallocation for typical 100 KiB - 1 MiB SERP pages
    let mut out: Vec<u8> = Vec::with_capacity(
        resp.content_length()
            .map(|len| (len as usize).min(MAX_RESPONSE_BODY))
            .unwrap_or(0),
    );
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| Error::internal(engine, "failed to read response body"))?;
        if out.len() + chunk.len() > MAX_RESPONSE_BODY {
            return Err(Error::schema(engine, BODY_TOO_LARGE));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

/// True when a stored fixture is a real, parseable SERP snapshot rather than
/// a transient anti-bot or error page. `fetch_fixtures` decides this at
/// capture time by actually parsing the body (content validation) and records
/// it in `tests/fixtures/meta.json`; fixture tests consult that record instead
/// of sniffing for marker strings.
#[cfg(test)]
pub fn fixture_parses(name: &str) -> bool {
    let meta = include_str!("../../tests/fixtures/meta.json");
    match serde_json::from_str::<serde_json::Value>(meta) {
        Ok(m) => m
            .get(name)
            .and_then(|e| e.get("parsed"))
            .and_then(|p| p.as_bool())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Parse a response body that must be JSON. A body that is not valid JSON is
/// a schema deviation (the endpoint returned something other than its
/// contract), reported as [`crate::error::ErrorScope::Schema`].
pub fn parse_json_body(engine: &'static str, body: &[u8]) -> Result<serde_json::Value> {
    serde_json::from_slice(body).map_err(|_| Error::schema(engine, "invalid JSON response"))
}

/// Decode the original URL from a Brave proxied image URL
/// (`https://imgs.search.brave.com/...`): the path ends with
/// `g:ce/<base64>` and the remainder is a standard-base64 payload.
pub fn brave_b64_decode(src: &str) -> String {
    let Some(idx) = src.rfind("/g:ce/") else {
        return String::new();
    };
    let b64 = &src[idx + 6..];
    let padded = format!("{}{}", b64, "=".repeat((4 - b64.len() % 4) % 4));
    use base64::Engine;
    let dec = base64::engine::general_purpose::STANDARD
        .decode(padded.as_bytes())
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(padded.as_bytes()));
    match dec {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    }
}

/// Parse --width / --height from a style attribute (Brave images).
pub fn brave_dims(style: &str) -> (u32, u32) {
    let mut w = 0u32;
    let mut h = 0u32;
    for part in style.split(';') {
        let part = part.trim();
        if let Some(v) = part.strip_prefix("--width:") {
            w = v.trim().parse().unwrap_or(0);
        } else if let Some(v) = part.strip_prefix("--height:") {
            h = v.trim().parse().unwrap_or(0);
        }
    }
    (w, h)
}

/// Parse a Brave SSR result wrapper (news/videos share the same layout).
pub fn parse_brave_wrapper(node: &scraper::ElementRef) -> Option<RawResult> {
    let url = parse::attr(node, "a.l1", "href").unwrap_or_default();
    if !url.starts_with("http") {
        return None;
    }
    let title = parse::select_first_nonempty(node, "div.title")?;
    Some(RawResult {
        title,
        url,
        description: parse::select_text(node, "div.description").unwrap_or_default(),
        source: parse::select_text(node, ".site-name-content").unwrap_or_default(),
        published: parse::select_text(node, ".metadata")
            .or_else(|| parse::select_text(node, ".age-header")),
        thumbnail_url: parse::attr(node, "a.thumbnail img", "src").unwrap_or_default(),
        duration: parse::select_text(node, ".duration").unwrap_or_default(),
        engine: String::new(),
        position: 0,
        ..Default::default()
    })
}

/// Map a [`TimeRange`] to the one-letter time filter used by most engines'
/// URL parameters.
pub fn time_param(t: &TimeRange) -> &'static str {
    match t {
        TimeRange::Day => "d",
        TimeRange::Week => "w",
        TimeRange::Month => "m",
        TimeRange::Year => "y",
    }
}

/// Map a [`TimeRange`] to its equivalent age in minutes, as used by Bing's
/// `qft` filter (`filterui:age-lt<N>`).
pub fn bing_time_minutes(t: &TimeRange) -> &'static str {
    match t {
        TimeRange::Day => "1440",
        TimeRange::Week => "10080",
        TimeRange::Month => "44640",
        TimeRange::Year => "525600",
    }
}

/// Parse absolute (`%d.%m.%Y`, `%m/%d/%Y`) and relative ("N days ago",
/// multi-language) date strings into ISO-8601. Shared by the Bing/Yahoo
/// news parsers.
pub fn normalize_date(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let now = chrono::Utc::now();
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%d.%m.%Y") {
        return d.and_hms_opt(0, 0, 0).map(|t| t.to_string() + "Z");
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%m/%d/%Y") {
        return d.and_hms_opt(0, 0, 0).map(|t| t.to_string() + "Z");
    }
    // relative: "3 days ago", "2 Stunden", "1 jour", "5 giorni", "2 dias"
    let words = s.to_lowercase();
    let unit: &[(&str, &[&str])] = &[
        ("minute", &["minute", "min", "min."]),
        ("hour", &["hour", "hours", "hora", "stunde"]),
        ("day", &["day", "days", "dia", "dias", "día"]),
        ("week", &["week", "weeks", "semana", "woche"]),
        ("month", &["month", "months", "mes", "monat"]),
        ("year", &["year", "years", "año", "jahr"]),
    ];
    let num = s
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<i64>()
        .ok()?;
    let unit_name = unit
        .iter()
        .find_map(|(name, ws)| ws.iter().any(|w| words.contains(w)).then_some(*name))?;
    let dur = match unit_name {
        "minute" => chrono::Duration::minutes(num),
        "hour" => chrono::Duration::hours(num),
        "day" => chrono::Duration::days(num),
        "week" => chrono::Duration::weeks(num),
        "month" => chrono::Duration::days(num * 30),
        _ => chrono::Duration::days(num * 365),
    };
    Some((now - dur).to_rfc3339())
}

/// DuckDuckGo JSON verticals' `p` (safesearch) parameter:
/// Strict -> `1`, Moderate -> `-1`, Off -> `-2`.
pub fn ddg_safesearch(safesearch: crate::models::SafeSearch) -> &'static str {
    match safesearch {
        crate::models::SafeSearch::Strict => "1",
        crate::models::SafeSearch::Moderate => "-1",
        crate::models::SafeSearch::Off => "-2",
    }
}

/// Capitalized time-range value used inside the `f`/`df` parameters of
/// DuckDuckGo's JSON verticals (`time:Day`, `publishedAfter:Week`, ...).
pub fn ddg_time_value(t: &TimeRange) -> &'static str {
    match t {
        TimeRange::Day => "Day",
        TimeRange::Week => "Week",
        TimeRange::Month => "Month",
        TimeRange::Year => "Year",
    }
}

/// Fetch the DuckDuckGo vqd token for a query, cached per query.
pub async fn ddg_vqd(ctx: &EngineContext<'_>, query: &str) -> Result<String> {
    let key = query.to_string();
    if let Some(v) = ctx.shared.vqd_get(&key) {
        return Ok(v);
    }
    fetch_vqd_and_cache(ctx, query).await
}

/// Force-fetch a fresh DuckDuckGo vqd token, bypassing (and replacing) the
/// cached one - used when a signed JSON endpoint rejects the current token.
pub async fn ddg_vqd_fresh(ctx: &EngineContext<'_>, query: &str) -> Result<String> {
    ctx.shared.vqd_invalidate(query);
    fetch_vqd_and_cache(ctx, query).await
}

async fn fetch_vqd_and_cache(ctx: &EngineContext<'_>, query: &str) -> Result<String> {
    let url = parse::with_query("https://duckduckgo.com/", [("q", query)]);
    let resp = ctx.client.get(&url).await?;
    let body = read_body(resp, "duckduckgo").await?;
    let text = String::from_utf8_lossy(&body);
    let vqd = extract_vqd(&text)
        .ok_or_else(|| Error::blocked("duckduckgo", BlockDetails::BotDetection))?;
    ctx.shared.vqd_set(query, vqd.clone());
    Ok(vqd)
}

/// Browser-like header set for DuckDuckGo's signed JSON endpoints
/// (`i.js` / `news.js` / `v.js`), mirroring what duckduckgo.com's own
/// frontend sends; reduces spurious bot-detection responses.
pub fn ddg_json_headers() -> wreq::header::HeaderMap {
    use wreq::header::{self, HeaderValue};
    let mut h = wreq::header::HeaderMap::new();
    h.insert(header::ACCEPT, HeaderValue::from_static("*/*"));
    h.insert(
        header::ACCEPT_LANGUAGE,
        HeaderValue::from_static("en-US,en;q=0.5"),
    );
    h.insert(
        header::REFERER,
        HeaderValue::from_static("https://duckduckgo.com/"),
    );
    // Sec-Fetch-* and Sec-GPC are not constants in wreq's header module
    for (name, value) in [
        ("sec-fetch-dest", "empty"),
        ("sec-fetch-mode", "cors"),
        ("sec-fetch-site", "same-origin"),
        ("sec-gpc", "1"),
    ] {
        if let (Ok(name), Ok(value)) = (
            header::HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(value),
        ) {
            h.insert(name, value);
        }
    }
    h
}

/// Fetch one of DuckDuckGo's vqd-signed JSON verticals (`i.js`, `news.js`,
/// `v.js`). `build_url` receives the vqd token and returns the full request
/// URL. When the endpoint blocks or rate-limits the attempt (stale token,
/// rate window), the token is refreshed and the call retried exactly once;
/// genuine blocks then surface as usual.
pub async fn fetch_ddg_vertical(
    ctx: &EngineContext<'_>,
    engine: &'static str,
    build_url: impl Fn(&str) -> Result<String>,
) -> Result<serde_json::Value> {
    let vqd = ddg_vqd(ctx, &ctx.opts.query).await?;
    let url = build_url(&vqd)?;
    let resp = ctx.client.get_with_headers(&url, &ddg_json_headers()).await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => return Err(e),
    };
    match check_response(engine, &resp, MediaType::Json) {
        Ok(()) => {}
        // stale token / rate window: refresh and retry once
        Err(e)
            if matches!(
                e.kind(),
                crate::error::ErrorKind::Blocked(_)
                    | crate::error::ErrorKind::RateLimited { .. }
                    | crate::error::ErrorKind::MalformedPayload { .. }
            ) =>
        {
            let fresh = ddg_vqd_fresh(ctx, &ctx.opts.query).await?;
            let url = build_url(&fresh)?;
            let resp = ctx
                .client
                .get_with_headers(&url, &ddg_json_headers())
                .await?;
            check_response(engine, &resp, MediaType::Json)?;
            let body = read_body(resp, engine).await?;
            return parse_json_body(engine, &body);
        }
        Err(e) => return Err(e),
    }
    let body = read_body(resp, engine).await?;
    parse_json_body(engine, &body)
}

/// Extract the DuckDuckGo `vqd` token from the HTML of a search page.
pub fn extract_vqd(text: &str) -> Option<String> {
    for needle in ["vqd=\"", "vqd=", "vqd='"] {
        if let Some(idx) = text.find(needle) {
            let rest = &text[idx + needle.len()..];
            let end = rest.find(['"', '\'', '&', ' ']).unwrap_or(rest.len());
            if end > 0 && end < 128 {
                return Some(rest[..end].to_string());
            }
        }
    }
    None
}

/// Normalize a result URL: strip tracking params common to all engines.
pub fn clean_url(url: &str) -> String {
    let url = url.trim();
    if url.is_empty() {
        return String::new();
    }

    url.strip_prefix("//")
        .map(|u| format!("https://{u}"))
        .unwrap_or_else(|| url.to_string())
}

/// Parse a DDG (html.duckduckgo.com) page and return (results, answer).
pub fn parse_ddg_html(body: &str, engine: &str) -> (Vec<crate::models::RawResult>, Option<String>) {
    use crate::models::RawResult;
    let doc = parse::parse_html(body);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("div.web-result").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let title = parse::select_first_nonempty(&node, "h2 a");
        let href = parse::attr(&node, "h2 a", "href");
        let snippet = parse::select_text(&node, "a.result__snippet");
        if let (Some(title), Some(mut url)) = (title, href) {
            if url.contains("duckduckgo.com/y.js?") {
                continue;
            }
            url = parse::unwrap_ddg_url(&url);
            url = clean_url(&url);
            pos += 1;
            out.push(RawResult {
                title,
                url,
                description: snippet.unwrap_or_default(),
                engine: engine.to_string(),
                position: pos,
                ..Default::default()
            });
        }
    }
    let answer = parse::doc_text(&doc, "div#zero_click_abstract")
        .or_else(|| parse::doc_text(&doc, "div.zero-click"));
    (out, answer)
}

/// Fallback HTTP GET via the system `curl` binary.
///
/// Used only by bootstrap engines (annas_archive/google/qwant): their
/// anti-bot systems dynamically flag individual TLS fingerprints, so when
/// the embedded client's fingerprint is tarpitted, the system curl's
/// completely different stack still passes with valid cookies.
/// Returns `(status_code, body_bytes)`; follows redirects.
pub fn curl_get(
    url: &str,
    cookie_header: Option<&str>,
    timeout_secs: u64,
) -> Result<(u16, Vec<u8>)> {
    use std::process::{Command, Stdio};
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-L", "--max-redirs", "5"])
        .arg("-w")
        .arg("\n%{http_code}")
        .arg("--max-time")
        .arg(timeout_secs.to_string())
        .arg("-A")
        .arg("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/148.0.0.0 Safari/537.36");
    if let Some(c) = cookie_header {
        cmd.arg("-H").arg(format!("Cookie: {c}"));
    }
    cmd.arg("-H")
        .arg("Accept: text/html,application/xhtml+xml,application/json,*/*;q=0.8");
    cmd.arg(url);
    cmd.stdout(Stdio::piped()).stderr(Stdio::null());
    let out = cmd.output().map_err(|_| Error::network("curl"))?;
    if !out.status.success() && out.stdout.is_empty() {
        return Err(Error::network("curl"));
    }
    // last line = status code written by -w
    let split = out.stdout.iter().rposition(|&b| b == b'\n').unwrap_or(0);
    let (body, code) = out.stdout.split_at(split);
    let code_str: String = String::from_utf8_lossy(&code[if split > 0 { 1 } else { 0 }..])
        .trim()
        .to_string();
    let status: u16 = code_str.parse().unwrap_or(0);
    Ok((status, body.to_vec()))
}

/// Random alphanumeric token (`A-Za-z0-9-_`) used by Yahoo's URL tokens and
/// Bing's per-request `cvid` values.
pub fn random_token(len: usize) -> String {
    use rand::RngExt;
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| CHARS[rng.random_range(0..CHARS.len())] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorKind;

    #[test]
    fn classify_uses_http_semantics() {
        use wreq::header::HeaderMap;
        let s = |n: u16| wreq::StatusCode::from_u16(n).unwrap();
        let h = HeaderMap::new();
        let with = |hm: &mut HeaderMap, k: &'static str, v: &'static str| {
            hm.insert(k, wreq::header::HeaderValue::from_static(v));
        };
        assert!(
            classify(
                "t",
                s(200),
                &h,
                Some("text/html; charset=utf-8"),
                MediaType::Html
            )
            .is_ok()
        );
        assert!(classify("t", s(200), &h, None, MediaType::Html).is_ok());
        assert!(classify("t", s(200), &h, Some("text/html"), MediaType::Html).is_ok());
        assert!(
            classify(
                "t",
                s(200),
                &h,
                Some("application/xhtml+xml"),
                MediaType::Html
            )
            .is_ok()
        );
        assert!(classify("t", s(200), &h, Some("application/json"), MediaType::Json).is_ok());
        assert!(
            classify(
                "t",
                s(200),
                &h,
                Some("application/problem+json"),
                MediaType::Json
            )
            .is_ok()
        );
        assert!(classify("t", s(200), &h, Some("text/plain"), MediaType::Json).is_ok());
        assert!(classify("t", s(204), &h, Some("application/json"), MediaType::Json).is_ok());
        assert!(matches!(
            classify("t", s(403), &h, Some("text/html"), MediaType::Html),
            Err(Error {
                kind: ErrorKind::Blocked(_),
                ..
            })
        ));
        assert!(matches!(
            classify("t", s(429), &h, Some("text/html"), MediaType::Html),
            Err(Error {
                kind: ErrorKind::RateLimited { .. },
                ..
            })
        ));
        assert!(matches!(
            classify("t", s(404), &h, Some("text/html"), MediaType::Html),
            Err(Error {
                kind: ErrorKind::UpstreamUnavailable { .. },
                ..
            })
        ));
        assert!(matches!(
            classify("t", s(500), &h, None, MediaType::Html),
            Err(Error {
                kind: ErrorKind::UpstreamUnavailable { .. },
                ..
            })
        ));
        assert!(matches!(
            classify("t", s(200), &h, Some("text/html"), MediaType::Json),
            Err(Error {
                kind: ErrorKind::MalformedPayload { .. },
                ..
            })
        ));
        assert!(matches!(
            classify("t", s(200), &h, Some("application/xml"), MediaType::Json),
            Err(Error {
                kind: ErrorKind::MalformedPayload { .. },
                ..
            })
        ));
        // Header anti-bot signals beat the status code.
        let mut hm = HeaderMap::new();
        with(&mut hm, "cf-mitigated", "challenge");
        assert!(matches!(
            classify("t", s(200), &hm, Some("text/html"), MediaType::Html),
            Err(Error {
                kind: ErrorKind::Blocked(BlockDetails::Cloudflare),
                ..
            })
        ));
        let mut hm = HeaderMap::new();
        with(&mut hm, "x-datadome", "captcha");
        assert!(matches!(
            classify("t", s(200), &hm, Some("text/html"), MediaType::Html),
            Err(Error {
                kind: ErrorKind::Blocked(BlockDetails::Captcha),
                ..
            })
        ));
        let mut hm = HeaderMap::new();
        with(&mut hm, "cf-ray", "abc");
        assert!(matches!(
            classify("t", s(403), &hm, Some("text/html"), MediaType::Html),
            Err(Error {
                kind: ErrorKind::Blocked(BlockDetails::Cloudflare),
                ..
            })
        ));
        let mut hm = HeaderMap::new();
        with(&mut hm, "retry-after", "120");
        let e = classify("t", s(429), &hm, Some("text/html"), MediaType::Html)
            .expect_err("must be an error");
        assert!(matches!(
            e.kind(),
            ErrorKind::RateLimited {
                retry_after: Some(d)
            } if d.as_secs() == 120
        ));
    }

    #[test]
    fn parse_json_body_classifies_blocks() {
        assert_eq!(parse_json_body("t", b"{}").unwrap(), serde_json::json!({}));
        assert!(parse_json_body("t", b"[1,2]").is_ok());
        assert!(matches!(
            parse_json_body("t", b""),
            Err(Error {
                kind: ErrorKind::MalformedPayload { .. },
                ..
            })
        ));
        assert!(matches!(
            parse_json_body("t", b"<html>Anomaly</html>"),
            Err(Error {
                kind: ErrorKind::MalformedPayload { .. },
                ..
            })
        ));
        assert!(matches!(
            parse_json_body("t", b"Anomaly detected, retry later"),
            Err(Error {
                kind: ErrorKind::MalformedPayload { .. },
                ..
            })
        ));
        assert!(matches!(
            parse_json_body("t", b"nope not json"),
            Err(Error {
                kind: ErrorKind::MalformedPayload { .. },
                ..
            })
        ));
        assert!(matches!(
            parse_json_body("t", b"{\"a\":1,"),
            Err(Error {
                kind: ErrorKind::MalformedPayload { .. },
                ..
            })
        ));
    }

    #[test]
    fn brave_b64_decodes_both_variants() {
        // padding-less input: function re-pads
        assert_eq!(brave_b64_decode("/g:ce/aGVsbG8"), "hello");
        // unpadded url-safe style with a '/'
        assert_eq!(
            brave_b64_decode(&format!("/g:ce/{}", "aHR0cHM6Ly9leGFtcGxlLmNvbS9pbWcucG5n")),
            "https://example.com/img.png"
        );
        assert_eq!(brave_b64_decode("no marker here"), "");
        assert_eq!(brave_b64_decode("/g:ce/%%%invalid%%%"), "");
    }

    #[test]
    fn brave_dims_parse() {
        assert_eq!(brave_dims("--width:250;--height:300"), (250, 300));
        assert_eq!(brave_dims("--width: 0"), (0, 0));
        assert_eq!(brave_dims(""), (0, 0));
    }

    #[test]
    fn vqd_extraction() {
        assert_eq!(extract_vqd("vqd=\"abc123\";x"), Some("abc123".into()));
        assert_eq!(extract_vqd("var vqd='xy z'"), Some("xy".into()));
        assert_eq!(extract_vqd("no token here"), None);
    }

    #[test]
    fn clean_url_handles_protocol_relative() {
        assert_eq!(clean_url("//example.com/a"), "https://example.com/a");
        assert_eq!(
            clean_url("  https://example.com/a  "),
            "https://example.com/a"
        );
        assert_eq!(clean_url(""), "");
    }

    #[test]
    fn time_and_safe_params() {
        assert_eq!(time_param(&TimeRange::Day), "d");
        assert_eq!(bing_time_minutes(&TimeRange::Week), "10080");
    }
}
