use scraper::{ElementRef, Html, Selector};

/// Collapse all whitespace runs into single spaces and trim.
pub fn collapse(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            space = true;
        } else {
            if space && !out.is_empty() {
                out.push(' ');
            }
            space = false;
            out.push(c);
        }
    }
    out.trim().to_string()
}

pub fn text_of(el: &ElementRef) -> String {
    collapse(&el.text().collect::<String>())
}

pub fn attr(el: &ElementRef, selector: &str, name: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    el.select(&sel)
        .next()?
        .value()
        .attr(name)
        .map(|s| s.to_string())
}

pub fn select_text(el: &ElementRef, selector: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    el.select(&sel)
        .next()
        .map(|e| text_of(&e))
        .filter(|t| !t.is_empty())
}

pub fn select_texts(el: &ElementRef, selector: &str) -> Vec<String> {
    let Ok(sel) = Selector::parse(selector) else {
        return Vec::new();
    };
    el.select(&sel).map(|e| text_of(&e)).collect()
}

/// Join the text of all matches of `selector` with `sep`.
pub fn select_text_joined(el: &ElementRef, selector: &str, sep: &str) -> String {
    let parts: Vec<String> = select_texts(el, selector)
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect();
    parts.join(sep)
}

/// First element whose own text content is non-empty.
pub fn select_first_nonempty(el: &ElementRef, selector: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    for node in el.select(&sel) {
        let t = text_of(&node);
        if !t.is_empty() {
            return Some(t);
        }
    }
    None
}

pub fn parse_html(html: &str) -> Html {
    Html::parse_document(html)
}

pub fn doc_text(doc: &Html, selector: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    doc.select(&sel)
        .next()
        .map(|e| text_of(&e))
        .filter(|t| !t.is_empty())
}

pub fn doc_attr(doc: &Html, selector: &str, name: &str) -> Option<String> {
    let sel = Selector::parse(selector).ok()?;
    doc.select(&sel)
        .next()?
        .value()
        .attr(name)
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Redirect unwrapping
// ---------------------------------------------------------------------------

/// `https://www.google.com/url?q=<url>&...`
pub fn unwrap_google_url(href: &str) -> String {
    if href.starts_with("/url?q=")
        || href.starts_with("http://www.google.com/url?q=")
        || href.starts_with("https://www.google.com/url?q=")
    {
        let q = href.split_once("?q=").map(|(_, r)| r).unwrap_or(href);
        let url = q.split('&').next().unwrap_or(q);
        return percent_decode(url);
    }
    href.to_string()
}

/// `https://www.bing.com/ck/a?u=a1<base64url>` (a1-prefixed payload).
pub fn unwrap_bing_url(href: &str) -> String {
    if let Some(idx) = href.find("u=a1") {
        let enc = &href[idx + 4..];
        let enc = enc.split('&').next().unwrap_or(enc);
        if let Ok(dec) = decode_b64url(enc) {
            return String::from_utf8_lossy(&dec).into_owned();
        }
    }
    href.to_string()
}

/// `//duckduckgo.com/l/?uddg=<urlencoded>`
pub fn unwrap_ddg_url(href: &str) -> String {
    if href.contains("duckduckgo.com/l/") && href.contains("uddg=") {
        let q = href.split_once("uddg=").map(|(_, r)| r).unwrap_or(href);
        return percent_decode(q.split('&').next().unwrap_or(q));
    }
    href.to_string()
}

/// Yahoo `/RU=<enc>/RK=2/RS=...` redirects.
pub fn unwrap_yahoo_url(href: &str) -> String {
    if href.contains("/RU=") {
        let after = href.split("/RU=").last().unwrap_or(href);
        let end = after
            .find("/RK=")
            .or_else(|| after.find("/RS="))
            .unwrap_or(after.len());
        return percent_decode(&after[..end]);
    }
    href.to_string()
}

/// Generic redirect wrapper: `http://example.com/?url=...` variants.
pub fn unwrap_wrapper_url(href: &str) -> String {
    for marker in ["?url=", "&url=", "?u=", "&u=", "?redirect="] {
        if let Some(idx) = href.find(marker) {
            let val = &href[idx + marker.len()..];
            let val = val.split('&').next().unwrap_or(val);
            if val.starts_with("http") {
                return percent_decode(val);
            }
        }
    }
    href.to_string()
}

pub fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s)
        .decode_utf8_lossy()
        .into_owned()
}

fn decode_b64url(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s)
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

pub fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .map(|u| u.host_str().unwrap_or("").to_string())
}

/// Append a query parameter to a URL string.
pub fn with_query<I, K, V>(url: &str, params: I) -> String
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    let mut u =
        url::Url::parse(url).unwrap_or_else(|_| url::Url::parse("https://invalid").unwrap());
    let mut q: Vec<(String, String)> = u
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    for (k, v) in params {
        q.push((k.as_ref().to_string(), v.as_ref().to_string()));
    }
    u.set_query(None);
    u.set_query(Some(
        &q.iter()
            .map(|(k, v)| format!("{}={}", encode_query(k), encode_query(v)))
            .collect::<Vec<_>>()
            .join("&"),
    ));
    u.to_string()
}

pub fn encode_query(s: &str) -> String {
    use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
    utf8_percent_encode(s, NON_ALPHANUMERIC).to_string()
}

/// application/x-www-form-urlencoded (spaces as `+`).
pub fn form_encode<I, K, V>(params: I) -> String
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<str>,
    V: AsRef<str>,
{
    params
        .into_iter()
        .map(|(k, v)| format!("{}={}", form_enc(k.as_ref()), form_enc(v.as_ref())))
        .collect::<Vec<_>>()
        .join("&")
}

fn form_enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(*b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Text snippet around the first occurrence of `needle`.
pub fn excerpt(text: &str, needle: &str, radius: usize) -> String {
    let lower = text.to_lowercase();
    let needle = needle.to_lowercase();
    if needle.is_empty() {
        return truncate(text, radius * 2);
    }
    match lower.find(&needle) {
        Some(pos) => {
            let start = pos.saturating_sub(radius);
            let end = (pos + needle.len() + radius).min(text.len());
            let mut out = text[start..end].trim().to_string();
            if start > 0 {
                out = format!("...{out}");
            }
            if end < text.len() {
                out.push_str("...");
            }
            out
        }
        None => truncate(text, radius * 2),
    }
}

pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let keep = max.saturating_sub(3);
    let cut: String = text.chars().take(keep).collect();
    if keep == 0 { cut } else { format!("{cut}...") }
}
