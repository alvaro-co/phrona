use std::collections::HashMap;

use crate::models::RawResult;

/// Normalize a URL for cross-engine deduplication.
pub fn dedup_key(url: &str) -> String {
    let Ok(u) = url::Url::parse(url) else {
        return url.to_lowercase();
    };
    let scheme = u.scheme().to_string();
    let host = u.host_str().unwrap_or("").to_lowercase();
    let mut path = u.path().trim_end_matches('/').to_string();
    if path.is_empty() {
        path = "/".to_string();
    }
    // strip common tracking parameters
    let strip: &[&str] = &[
        "utm_source",
        "utm_medium",
        "utm_campaign",
        "utm_term",
        "utm_content",
        "fbclid",
        "gclid",
        "gclsrc",
        "dclid",
        "msclkid",
        "mc_eid",
        "igshid",
        "ref",
        "source",
        "si",
        "spm",
        "s",
        "yclid",
    ];
    let query: Vec<(String, String)> = u
        .query_pairs()
        .map(|(k, v)| (k.to_lowercase(), v.into_owned()))
        .filter(|(k, _)| !strip.contains(&k.as_str()))
        .collect();
    let mut key = format!("{scheme}://{host}{path}");
    if !query.is_empty() {
        let mut q: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        q.sort();
        key.push_str(&format!("?{}", q.join("&")));
    }
    key
}

#[derive(Debug, Clone)]
pub struct GroupedResult {
    pub result: RawResult,
    pub engines: Vec<String>,
    pub count: usize,
}

/// Merge raw results from many engines: group by dedup key.
/// Results with an empty url (answer markers) are dropped here.
pub fn group<I>(all: I) -> Vec<GroupedResult>
where
    I: IntoIterator<Item = RawResult>,
{
    let mut map: HashMap<String, GroupedResult> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    for raw in all {
        if raw.url.is_empty() {
            continue;
        }
        let key = dedup_key(&raw.url);
        match map.get_mut(&key) {
            Some(g) => {
                g.count += 1;
                if !g.engines.contains(&raw.engine) {
                    g.engines.push(raw.engine.clone());
                }
            }
            None => {
                order.push(key.clone());
                map.insert(
                    key,
                    GroupedResult {
                        engines: vec![raw.engine.clone()],
                        count: 1,
                        result: raw,
                    },
                );
            }
        }
    }
    order.into_iter().filter_map(|k| map.remove(&k)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(title: &str, url: &str, engine: &str) -> RawResult {
        RawResult {
            title: title.into(),
            url: url.into(),
            engine: engine.into(),
            ..Default::default()
        }
    }

    #[test]
    fn dedup_key_normalizes() {
        assert_eq!(dedup_key("HTTP://Example.COM/a/"), "http://example.com/a");
        assert_eq!(dedup_key("https://example.com/"), "https://example.com/");
        assert_eq!(dedup_key("https://example.com/a/"), "https://example.com/a");
    }

    #[test]
    fn dedup_key_strips_tracking_params() {
        assert_eq!(
            dedup_key("https://example.com/a?utm_source=x&utm_medium=y&id=1"),
            "https://example.com/a?id=1"
        );
        assert_eq!(
            dedup_key("https://example.com/a?fbclid=abc&si=def&ref=ghi"),
            "https://example.com/a"
        );
    }

    #[test]
    fn dedup_key_strips_uppercase_tracking_params() {
        // Tracking parameters are conventionally case-insensitive; an
        // engine returning UTM/GCLID in uppercase must dedupe against one
        // returning them lowercase.
        assert_eq!(
            dedup_key("https://example.com/a?UTM_SOURCE=x&GCLID=123&id=1"),
            "https://example.com/a?id=1"
        );
        assert_eq!(
            dedup_key("https://example.com/a?utm_source=x&gclid=123"),
            "https://example.com/a"
        );
        assert_eq!(
            dedup_key("https://example.com/a?UTM_SOURCE=x"),
            dedup_key("https://example.com/a?utm_source=x")
        );
    }

    #[test]
    fn dedup_key_sorts_remaining_query() {
        assert_eq!(
            dedup_key("https://example.com/a?b=2&a=1"),
            "https://example.com/a?a=1&b=2"
        );
    }

    #[test]
    fn dedup_key_unparseable_falls_back() {
        assert_eq!(dedup_key("not a url"), "not a url");
    }

    #[test]
    fn group_merges_across_engines() {
        let items = vec![
            raw("A", "https://example.com/a?utm_source=x", "bing"),
            raw("A", "https://example.com/a", "brave"),
            raw("B", "https://example.org/b", "brave"),
        ];
        let g = group(items);
        assert_eq!(g.len(), 2);
        let a = g.iter().find(|g| g.result.title == "A").unwrap();
        assert_eq!(a.count, 2);
        assert_eq!(a.engines, ["bing", "brave"]);
    }

    #[test]
    fn group_drops_answer_markers_and_empty_urls() {
        let items = vec![
            raw("answer", "", "grokipedia"),
            raw("A", "https://example.com/a", "bing"),
        ];
        assert_eq!(group(items).len(), 1);
    }
}
