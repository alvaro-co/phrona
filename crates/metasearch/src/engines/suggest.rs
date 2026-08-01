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
    match source {
        SuggestSource::DuckDuckGo => {
            let url = parse::with_query(
                "https://duckduckgo.com/ac/",
                [("type", "list"), ("q", query), ("kl", region)],
            );
            let resp = client.get(&url).await?;
            let body = resp.bytes().await.map_err(Error::from)?;
            let json: serde_json::Value = serde_json::from_slice(&body)
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
            let (lang, _) = region
                .split_once('-')
                .map(|(l, _)| (l.to_string(), ()))
                .unwrap_or((region.to_string(), ()));
            let url = parse::with_query(
                "https://www.google.com/complete/search",
                [("q", query), ("client", "gws-wiz"), ("hl", lang.as_str())],
            );
            let resp = client.get(&url).await?;
            let body = resp.bytes().await.map_err(Error::from)?;
            let text = String::from_utf8_lossy(&body);
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
                        let doc = parse::parse_html(html);
                        let sel = scraper::Selector::parse("b").unwrap_or_else(|_| unreachable!());
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
            let cvid = crate::engines::util::random_token(32);
            let url = parse::with_query(
                "https://www.bing.com/AS/Suggestions",
                [("qry", query), ("csr", "1"), ("cvid", cvid.as_str())],
            );
            let resp = client.get(&url).await?;
            let body = resp.bytes().await.map_err(Error::from)?;
            let json: serde_json::Value =
                serde_json::from_slice(&body).map_err(|e| Error::Parse(e.to_string()))?;
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
        SuggestSource::Brave => {
            let url = parse::with_query("https://search.brave.com/api/suggest", [("q", query)]);
            let resp = client.get(&url).await?;
            let body = resp.bytes().await.map_err(Error::from)?;
            let json: serde_json::Value =
                serde_json::from_slice(&body).map_err(|e| Error::Parse(e.to_string()))?;
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
        SuggestSource::Startpage => {
            let url = parse::with_query(
                "https://www.startpage.com/suggestions",
                [
                    ("q", query),
                    ("format", "opensearch"),
                    ("segment", "startpage.defaultffx"),
                ],
            );
            let resp = client.get(&url).await?;
            let body = resp.bytes().await.map_err(Error::from)?;
            let json: serde_json::Value =
                serde_json::from_slice(&body).map_err(|e| Error::Parse(e.to_string()))?;
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
            let url = parse::with_query(
                "https://api.qwant.com/v3/suggest",
                [("q", query), ("locale", "en_US"), ("version", "2")],
            );
            let resp = client.get(&url).await?;
            let body = resp.bytes().await.map_err(Error::from)?;
            let json: serde_json::Value =
                serde_json::from_slice(&body).map_err(|e| Error::Parse(e.to_string()))?;
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
            let url = parse::with_query(
                "https://en.wikipedia.org/w/api.php",
                [
                    ("action", "opensearch"),
                    ("limit", "10"),
                    ("namespace", "0"),
                    ("format", "json"),
                    ("search", query),
                ],
            );
            let resp = client.get(&url).await?;
            let body = resp.bytes().await.map_err(Error::from)?;
            let json: serde_json::Value =
                serde_json::from_slice(&body).map_err(|e| Error::Parse(e.to_string()))?;
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
