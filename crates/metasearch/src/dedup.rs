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
        .filter(|(k, _)| !strip.contains(&k.as_ref()))
        .map(|(k, v)| (k.to_lowercase(), v.into_owned()))
        .collect();
    let mut key = format!("{scheme}://{host}{path}");
    if !query.is_empty() {
        let mut q: Vec<String> = query.iter().map(|(k, v)| format!("{k}={v}")).collect();
        q.sort();
        key.push_str(&format!("?{}", q.join("&")));
    }
    key
}

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
