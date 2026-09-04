//! Autocomplete suggestions from browser/engine sources.

use crate::client::HttpClient;
use crate::error::{Error, Result};
use crate::parse;

/// Autocomplete sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestSource {
    /// DuckDuckGo suggestions.
    DuckDuckGo,
    /// Google suggestions.
    Google,
    /// Bing suggestions.
    Bing,
    /// Brave suggestions.
    Brave,
    /// Startpage suggestions.
    Startpage,
    /// Qwant suggestions.
    Qwant,
    /// Wikipedia suggestions.
    Wikipedia,
}

impl SuggestSource {
    /// All [`SuggestSource`] variants, in a stable order.
    pub const ALL: [SuggestSource; 7] = [
        SuggestSource::DuckDuckGo,
        SuggestSource::Google,
        SuggestSource::Bing,
        SuggestSource::Brave,
        SuggestSource::Startpage,
        SuggestSource::Qwant,
        SuggestSource::Wikipedia,
    ];

    /// Return the source's stable name, e.g. `"duckduckgo"`.
    pub fn name(&self) -> &'static str {
        match self {
            SuggestSource::DuckDuckGo => "duckduckgo",
            SuggestSource::Google => "google",
            SuggestSource::Bing => "bing",
            SuggestSource::Brave => "brave",
            SuggestSource::Startpage => "startpage",
            SuggestSource::Qwant => "qwant",
            SuggestSource::Wikipedia => "wikipedia",
        }
    }

    /// Look up a [`SuggestSource`] by its [`name`][`SuggestSource::name`],
    /// returning `None` for unknown names.
    pub fn from_name(name: &str) -> Option<SuggestSource> {
        Self::ALL.iter().copied().find(|s| s.name() == name)
    }
}

/// Fetch autocomplete suggestions for `query` from a single [`SuggestSource`].
pub async fn suggest(
    client: &HttpClient,
    source: SuggestSource,
    query: &str,
    region: &str,
) -> Result<Vec<String>> {
    let body = fetch(client, source, query, region).await?;
    parse(source, &body)
}

async fn fetch(
    client: &HttpClient,
    source: SuggestSource,
    query: &str,
    region: &str,
) -> Result<Vec<u8>> {
    let url = match source {
        SuggestSource::DuckDuckGo => parse::with_query(
            "https://duckduckgo.com/ac/",
            [("type", "list"), ("q", query), ("kl", region)],
        ),
        SuggestSource::Google => {
            // regions are `country-lang` (`us-en`): the language is the
            // part AFTER the dash
            let lang = region.split_once('-').map(|(_, l)| l).unwrap_or(region);
            parse::with_query(
                "https://www.google.com/complete/search",
                [("q", query), ("client", "gws-wiz"), ("hl", lang)],
            )
        }
        SuggestSource::Bing => {
            let cvid = crate::engines::util::random_token(32);
            parse::with_query(
                "https://www.bing.com/AS/Suggestions",
                [("qry", query), ("csr", "1"), ("cvid", cvid.as_str())],
            )
        }
        SuggestSource::Brave => {
            parse::with_query("https://search.brave.com/api/suggest", [("q", query)])
        }
        SuggestSource::Startpage => parse::with_query(
            "https://www.startpage.com/suggestions",
            [
                ("q", query),
                ("format", "opensearch"),
                ("segment", "startpage.defaultffx"),
            ],
        ),
        SuggestSource::Qwant => {
            // Qwant expects an uppercase-country locale (`en_US`); fall back
            // to `en_US` when the region is not a `lang-country` pair.
            let locale = qwant_locale(region);
            parse::with_query(
                "https://api.qwant.com/v3/suggest",
                [("q", query), ("locale", locale.as_str()), ("version", "2")],
            )
        }
        SuggestSource::Wikipedia => {
            let lang = wikipedia_lang(region);
            parse::with_query(
                &format!("https://{lang}.wikipedia.org/w/api.php"),
                [
                    ("action", "opensearch"),
                    ("limit", "10"),
                    ("namespace", "0"),
                    ("format", "json"),
                    ("search", query),
                ],
            )
        }
    };
    let resp = client.get(&url).await?;
    crate::engines::util::check_response(
        source.name(),
        &resp,
        crate::engines::util::MediaType::Any,
    )?;
    crate::engines::util::read_body(resp, source.name()).await
}

/// Qwant API locale for a `lang-country` region (`us-en`/`en-us` -> `en_US`).
/// A bare language maps onto itself as the country (`de` -> `de_DE`);
/// an empty region falls back to `en_US`.
pub fn qwant_locale(region: &str) -> String {
    let region = region.trim();
    if region.is_empty() {
        return "en_US".into();
    }
    let (lang, country) = region
        .split_once('-')
        .map(|(country, lang)| (lang.trim(), country.trim()))
        .unwrap_or((region, region));
    let lang = if lang.is_empty() { "en" } else { lang };
    format!("{}_{}", lang.to_lowercase(), country.to_uppercase())
}

/// Wikipedia language subdomain for a region. Regions here follow the
/// engine convention `country-language` (`us-en`), but callers often pass
/// BCP47-style `language-country` (`ja-jp`), so both parts are tried
/// (language position first) against existing Wikipedia editions.
pub fn wikipedia_lang(region: &str) -> String {
    const EDITIONS: [&str; 24] = [
        "en", "de", "fr", "es", "it", "ru", "ja", "zh", "pt", "ar", "fa", "id", "tr", "ko", "nl",
        "pl", "sv", "uk", "vi", "cs", "hu", "ro", "fi", "no",
    ];
    let lower = region.trim().to_ascii_lowercase();
    let mut it = lower.split('-').filter(|p| !p.is_empty());
    let first = it.next();
    let second = it.next();
    // `country-language` convention puts the language second
    // (`us-en`); BCP47-style input (`ja-jp`) puts it first - try both.
    // The trailing `en` can never miss (`EDITIONS` always contains it),
    // but a safe fallback beats an `unreachable!` if the table is ever
    // edited without it.
    let candidates = [second, first, Some("en")];
    for c in candidates.into_iter().flatten() {
        if EDITIONS.contains(&c) {
            return c.to_string();
        }
    }
    "en".to_string()
}

/// Parse an autocomplete response body for a source. Pure function so
/// every source has an offline unit test.
pub fn parse(source: SuggestSource, body: &[u8]) -> Result<Vec<String>> {
    match source {
        SuggestSource::Google => {
            let text = String::from_utf8_lossy(body);
            let start = text
                .find('[')
                .ok_or_else(|| Error::schema(source.name(), "malformed suggest body"))?;
            let end = text
                .rfind(']')
                .ok_or_else(|| Error::schema(source.name(), "malformed suggest body"))?;
            let json: serde_json::Value = serde_json::from_str(&text[start..=end])
                .map_err(|_| Error::schema(source.name(), "invalid JSON response"))?;
            let mut out = Vec::new();
            // one static selector for the whole response, not one per item
            static BODY: std::sync::OnceLock<scraper::Selector> = std::sync::OnceLock::new();
            let sel = BODY.get_or_init(|| scraper::Selector::parse("body").unwrap());
            if let Some(items) = json.get(0).and_then(|v| v.as_array()) {
                for item in items {
                    if let Some(html) = item.get(0).and_then(|v| v.as_str()) {
                        // the suggestion may carry <b> emphasis; take the full
                        // text so prefixes are not lost
                        let doc = parse::parse_html(html);
                        if let Some(b) = doc.select(sel).next() {
                            out.push(parse::text_of(&b));
                        } else {
                            out.push(parse::collapse(html));
                        }
                    }
                }
            }
            Ok(out)
        }
        SuggestSource::Bing => {
            let json: serde_json::Value = serde_json::from_slice(body)
                .map_err(|_| Error::schema(source.name(), "invalid JSON response"))?;
            Ok(json
                .get("s")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| {
                            s.get("q")
                                .and_then(|q| q.as_str())
                                .map(|q| q.replace(['\u{e000}', '\u{e001}'], ""))
                        })
                        .collect()
                })
                .unwrap_or_default())
        }
        SuggestSource::Qwant => {
            let json: serde_json::Value = serde_json::from_slice(body)
                .map_err(|_| Error::schema(source.name(), "invalid JSON response"))?;
            Ok(json
                .pointer("/data/items")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.get("value").and_then(|v| v.as_str()).map(String::from))
                        .collect()
                })
                .unwrap_or_default())
        }
        // DuckDuckGo, Brave, Startpage and Wikipedia all answer the same
        // opensearch shape (`[query, [suggestions...], ...]`); a shared
        // arm also keeps future sources working if they follow it.
        _ => {
            let json: serde_json::Value = serde_json::from_slice(body)
                .map_err(|_| Error::schema(source.name(), "invalid JSON response"))?;
            Ok(json
                .get(1)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default())
        }
    }
}

/// Query every source in parallel; returns (source, suggestions) pairs.
pub async fn suggest_all(
    client: &HttpClient,
    query: &str,
    region: &str,
) -> Vec<(SuggestSource, Vec<String>)> {
    let futs = SuggestSource::ALL.iter().copied().map(|source| async move {
        (
            source,
            suggest(client, source, query, region)
                .await
                .unwrap_or_default(),
        )
    });
    let mut out: Vec<_> = futures::future::join_all(futs).await;
    out.sort_by_key(|(s, _)| s.name());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_names_roundtrip() {
        for s in SuggestSource::ALL {
            assert_eq!(SuggestSource::from_name(s.name()), Some(s));
        }
        assert_eq!(SuggestSource::from_name("nope"), None);
    }

    #[test]
    fn locales_derive_from_region() {
        assert_eq!(qwant_locale("us-en"), "en_US");
        assert_eq!(qwant_locale("de-de"), "de_DE");
        assert_eq!(qwant_locale("de"), "de_DE");
        assert_eq!(qwant_locale(""), "en_US");
        assert_eq!(wikipedia_lang("de-de"), "de");
        assert_eq!(wikipedia_lang("xx-yy"), "en");
        assert_eq!(wikipedia_lang("ja-jp"), "ja");
    }

    #[test]
    fn parse_duckduckgo() {
        let body = br#"["rus",["rust","russian","rustdesk"],["",""],[""]]"#;
        let out = parse(SuggestSource::DuckDuckGo, body).unwrap();
        assert_eq!(out, ["rust", "russian", "rustdesk"]);
        assert!(parse(SuggestSource::DuckDuckGo, b"not json").is_err());
    }

    #[test]
    fn parse_google() {
        let body =
            br#"window.google.ac.h([[["rust","rust lang",["x"]],["russian",["y"]]],"rus",{}])"#;
        let out = parse(SuggestSource::Google, body).unwrap();
        assert_eq!(out, ["rust", "russian"]);
    }

    #[test]
    fn parse_google_strips_bold_html() {
        // suggestion with <b> emphasis must be unmarked
        let body = br#"window.google.ac.h([[["ru<b>st</b>",0]],"rus",{}])"#;
        let out = parse(SuggestSource::Google, body).unwrap();
        assert_eq!(out, ["rust"]);
    }

    #[test]
    fn parse_bing() {
        let body = br#"{"s":[{"q":"rust\ue000lang\ue001"},{"q":"russian"}]}"#;
        let out = parse(SuggestSource::Bing, body).unwrap();
        assert_eq!(out, ["rustlang", "russian"]);
    }

    #[test]
    fn parse_brave_and_startpage() {
        for (src, body) in [
            (SuggestSource::Brave, br#"["rus",["rust","russian"]]"#),
            (SuggestSource::Startpage, br#"["rus",["rust","russian"]]"#),
        ] {
            let out = parse(src, body).unwrap();
            assert_eq!(out, ["rust", "russian"]);
        }
    }

    #[test]
    fn parse_qwant() {
        let body =
            br#"{"status":"success","data":{"items":[{"value":"rust"},{"value":"russian"}]}}"#;
        let out = parse(SuggestSource::Qwant, body).unwrap();
        assert_eq!(out, ["rust", "russian"]);
    }

    #[test]
    fn parse_wikipedia() {
        let body = br#"["rus",["rust","russian"],["",""]]"#;
        let out = parse(SuggestSource::Wikipedia, body).unwrap();
        assert_eq!(out, ["rust", "russian"]);
    }

    #[test]
    fn malformed_bodies_yield_empty_or_error() {
        // structurally valid JSON without suggestions -> empty, not error
        assert_eq!(
            parse(SuggestSource::DuckDuckGo, b"[]").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse(SuggestSource::Bing, b"{}").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            parse(SuggestSource::Qwant, b"{}").unwrap(),
            Vec::<String>::new()
        );
        assert!(parse(SuggestSource::Wikipedia, b"garbage").is_err());
    }
}
