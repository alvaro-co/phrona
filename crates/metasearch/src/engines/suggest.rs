use crate::client::HttpClient;
use crate::error::{Error, Result};
use crate::parse;

/// Autocomplete sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestSource {
    DuckDuckGo,
    Google,
    Bing,
    Brave,
    Startpage,
    Qwant,
    Wikipedia,
}

impl SuggestSource {
    pub const ALL: [SuggestSource; 7] = [
        SuggestSource::DuckDuckGo,
        SuggestSource::Google,
        SuggestSource::Bing,
        SuggestSource::Brave,
        SuggestSource::Startpage,
        SuggestSource::Qwant,
        SuggestSource::Wikipedia,
    ];

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

    pub fn from_name(name: &str) -> Option<SuggestSource> {
        Self::ALL.iter().copied().find(|s| s.name() == name)
    }
}

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
            let (lang, _) = region
                .split_once('-')
                .map(|(l, _)| (l.to_string(), ()))
                .unwrap_or((region.to_string(), ()));
            parse::with_query(
                "https://www.google.com/complete/search",
                [("q", query), ("client", "gws-wiz"), ("hl", lang.as_str())],
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
        SuggestSource::Qwant => parse::with_query(
            "https://api.qwant.com/v3/suggest",
            [("q", query), ("locale", "en_US"), ("version", "2")],
        ),
        SuggestSource::Wikipedia => parse::with_query(
            "https://en.wikipedia.org/w/api.php",
            [
                ("action", "opensearch"),
                ("limit", "10"),
                ("namespace", "0"),
                ("format", "json"),
                ("search", query),
            ],
        ),
    };
    let resp = client.get(&url).await?;
    Ok(resp.bytes().await.map_err(Error::from)?.to_vec())
}

/// Parse an autocomplete response body for a source. Pure function so
/// every source has an offline unit test.
pub fn parse(source: SuggestSource, body: &[u8]) -> Result<Vec<String>> {
    match source {
        SuggestSource::DuckDuckGo => {
            let json: serde_json::Value = serde_json::from_slice(body)
                .map_err(|e| Error::Parse(format!("ddg suggest: {e}")))?;
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
        SuggestSource::Google => {
            let text = String::from_utf8_lossy(body);
            let start = text
                .find('[')
                .ok_or_else(|| Error::Parse("google suggest".into()))?;
            let end = text
                .rfind(']')
                .ok_or_else(|| Error::Parse("google suggest".into()))?;
            let json: serde_json::Value = serde_json::from_str(&text[start..=end])
                .map_err(|e| Error::Parse(e.to_string()))?;
            let mut out = Vec::new();
            if let Some(items) = json.get(0).and_then(|v| v.as_array()) {
                for item in items {
                    if let Some(html) = item.get(0).and_then(|v| v.as_str()) {
                        // the suggestion may carry <b> emphasis; take the full
                        // text so prefixes are not lost
                        let doc = parse::parse_html(html);
                        let sel =
                            scraper::Selector::parse("body").unwrap_or_else(|_| unreachable!());
                        if let Some(b) = doc.select(&sel).next() {
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
            let json: serde_json::Value =
                serde_json::from_slice(body).map_err(|e| Error::Parse(e.to_string()))?;
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
        SuggestSource::Brave | SuggestSource::Startpage => {
            let json: serde_json::Value =
                serde_json::from_slice(body).map_err(|e| Error::Parse(e.to_string()))?;
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
        SuggestSource::Qwant => {
            let json: serde_json::Value =
                serde_json::from_slice(body).map_err(|e| Error::Parse(e.to_string()))?;
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
        SuggestSource::Wikipedia => {
            let json: serde_json::Value =
                serde_json::from_slice(body).map_err(|e| Error::Parse(e.to_string()))?;
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
