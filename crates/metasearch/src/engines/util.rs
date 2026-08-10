use crate::engine::EngineContext;
use crate::error::{Error, Result};
use crate::models::{RawResult, SafeSearch, TimeRange};
use crate::parse;

/// Common helpers shared by engines.
///
/// True when a captured page is a bot-block / captcha / error page rather
/// than a real SERP. Used by fixture tests and the capture tool.
pub fn is_block_page(text: &str) -> bool {
    let t = text.to_lowercase();
    t.contains("enablejs")
        || t.contains("/sorry/")
        || t.contains("captcha-delivery")
        || t.contains("403 - forbidden")
        || t.contains("anomaly")
        || t.contains("there are no search results")
        || t.contains("too many requests")
        || t.contains("page requires javascript")
        || t.contains("enable javascript")
        || t.contains("solvesimplechallenge")
        || (t.contains("<!doctype html") && t.contains("retry/enablejs"))
}

/// Decode the original URL from a Brave proxied image URL
/// (https://imgs.search.brave.com/.../g:ce/<base64>).
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

pub fn time_param(t: &TimeRange) -> &'static str {
    match t {
        TimeRange::Day => "d",
        TimeRange::Week => "w",
        TimeRange::Month => "m",
        TimeRange::Year => "y",
    }
}

pub fn bing_time_minutes(t: &TimeRange) -> &'static str {
    match t {
        TimeRange::Day => "1440",
        TimeRange::Week => "10080",
        TimeRange::Month => "44640",
        TimeRange::Year => "525600",
    }
}

pub fn safe_param(
    ss: SafeSearch,
    on: &'static str,
    off: &'static str,
    _mod: &'static str,
) -> &'static str {
    match ss {
        SafeSearch::Strict => on,
        SafeSearch::Off => off,
        SafeSearch::Moderate => _mod,
    }
}

/// Fetch the DuckDuckGo vqd token for a query, cached per query.
pub async fn ddg_vqd(ctx: &EngineContext<'_>, query: &str) -> Result<String> {
    let key = query.to_string();
    if let Some(v) = ctx.shared.vqd_get(&key).await {
        return Ok(v);
    }
    let url = parse::with_query("https://duckduckgo.com/", [("q", query)]);
    let resp = ctx.client.get(&url).await?;
    let body = resp.bytes().await.map_err(|e| Error::Http(e.to_string()))?;
    let text = String::from_utf8_lossy(&body);
    let vqd = extract_vqd(&text).ok_or_else(|| {
        Error::Engine(format!(
            "duckduckgo: vqd token not found (blocked?) query={query}"
        ))
    })?;
    ctx.shared.vqd_set(&key, vqd.clone()).await;
    Ok(vqd)
}

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

/// Random 16-char alphanumeric token (used by yahoo url tokens).
pub fn random_token(len: usize) -> String {
    use rand::Rng;
    let chars: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rng = rand::rng();
    (0..len)
        .map(|_| chars[rng.random_range(0..chars.len())] as char)
        .collect()
}

/// Convert a JS object literal to JSON (single quotes, unquoted keys).
pub fn js_to_json(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    let mut in_str = false;
    let mut str_q = '"';
    while let Some(c) = chars.next() {
        if in_str {
            if c == '\\' {
                out.push(c);
                if let Some(n) = chars.next() {
                    out.push(n);
                }
                continue;
            }
            if c == str_q {
                in_str = false;
                // convert closing single quotes to double quotes so the
                // output stays valid JSON
                out.push(if str_q == '\'' { '"' } else { c });
            } else {
                out.push(c);
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                in_str = true;
                str_q = c;
                out.push('"');
            }
            c if c.is_ascii_alphanumeric() || c == '_' || c == '$' => {
                let mut ident = String::new();
                ident.push(c);
                while let Some(&n) = chars.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' || n == '$' {
                        ident.push(n);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let is_num = ident.chars().all(|c| c.is_ascii_digit());
                let lit = matches!(ident.as_str(), "true" | "false" | "null");
                if !lit && !is_num {
                    out.push('"');
                    out.push_str(&ident);
                    out.push('"');
                } else {
                    out.push_str(&ident);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Extract the embedded JSON payload from a Brave SERP page.
pub fn extract_brave_json(html: &str) -> Option<serde_json::Value> {
    let start = html.find("<script")?;
    let text = &html[start..];
    let end = text.find("</script")?;
    let text = &text[..end];
    let data_start = text.find("data: [{")?;
    let data_end = text.rfind("}}]")?;
    let payload = format!("{{{}}}", &text[data_start..data_end]);
    let json = js_to_json(&payload);
    serde_json::from_str(&json).ok()
}

/// Locate the response object inside a Brave page payload:
/// data[i].data.body.response (searxng uses index 1, we search all).
pub fn brave_response(json: &serde_json::Value) -> Option<&serde_json::Value> {
    let data = json.get("data")?.as_array()?;
    for entry in data {
        if let Some(body) = entry
            .get("data")
            .and_then(|d| d.get("body"))
            .and_then(|b| b.get("response"))
        {
            return Some(body);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_page_detection() {
        for page in [
            "enablejs=1 and a captcha",
            "https://www.google.com/sorry/index?continue=...",
            "captcha-delivery.com",
            "403 - Forbidden by the server",
            "you are an anomaly",
            "there are no search results",
            "too many requests, retry later",
            "this page requires javascript to work",
            "please enable javascript",
            "window.SolveSimpleChallenge",
            "<!doctype html><html><script>retry/enablejs</script>",
        ] {
            assert!(is_block_page(page), "marker not detected: {page}");
        }
        for page in ["normal page with content", "", "error 404 not found here"] {
            assert!(!is_block_page(page), "false positive: {page}");
        }
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
        assert_eq!(safe_param(SafeSearch::Strict, "1", "0", "-1"), "1");
        assert_eq!(safe_param(SafeSearch::Off, "1", "0", "-1"), "0");
        assert_eq!(safe_param(SafeSearch::Moderate, "1", "0", "-1"), "-1");
    }

    #[test]
    fn js_to_json_quotes_keys() {
        let input = "{type: 'web', ok: true, n: 3, src: \"x\"}";
        let out = js_to_json(input);
        assert_eq!(
            out,
            "{\"type\": \"web\", \"ok\": true, \"n\": 3, \"src\": \"x\"}"
        );
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["type"], "web");
        assert_eq!(v["n"], 3);
    }
}
