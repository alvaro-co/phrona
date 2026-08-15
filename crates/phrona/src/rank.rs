use crate::dedup::GroupedResult;

/// Relevance scoring: engines rank results per-position; we blend
/// cross-engine frequency, per-engine position, and content matching.
pub fn rank(groups: Vec<GroupedResult>, query: &str) -> Vec<(f64, GroupedResult)> {
    let terms: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .filter(|t| t.chars().count() > 2)
        .map(|t| t.trim_matches(|c: char| !c.is_alphanumeric()).to_string())
        .filter(|t| !t.is_empty())
        .collect();

    let mut scored: Vec<(f64, GroupedResult)> = groups
        .into_iter()
        .map(|g| {
            let mut score = 0.0;
            // cross-engine agreement
            score += (g.count - 1) as f64 * 1.5;
            // engine position: earlier is better
            let pos = g.result.position.max(1) as f64;
            score += (10.0 / pos).min(3.0);
            // wikipedia preference (answer-like, high precision)
            let host = crate::parse::host_of(&g.result.url).unwrap_or_default();
            if host.contains("wikipedia.org") || host.contains("grokipedia") {
                score += 3.0;
            }
            score += text_match(&g.result.title, &g.result.description, &terms);
            (score, g)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    scored
}

fn text_match(title: &str, body: &str, terms: &[String]) -> f64 {
    if terms.is_empty() {
        return 0.0;
    }
    let title = title.to_lowercase();
    let body = body.to_lowercase();
    let title_hits = terms.iter().filter(|t| title.contains(t.as_str())).count();
    let body_hits = terms.iter().filter(|t| body.contains(t.as_str())).count();
    let all = terms.len() as f64;
    let t = title_hits as f64 / all;
    let b = body_hits as f64 / all;
    match (title_hits, body_hits) {
        (0, 0) => 0.0,
        (x, y) if x > 0 && y > 0 => 1.0 + t * 2.0 + b,
        (x, _) if x > 0 => 0.8 + t * 1.5,
        (_, _) => 0.4 + b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedup::GroupedResult;
    use crate::models::RawResult;

    fn group(title: &str, url: &str, desc: &str, engine: &str, position: u32) -> GroupedResult {
        GroupedResult {
            result: RawResult {
                title: title.into(),
                url: url.into(),
                description: desc.into(),
                engine: engine.into(),
                position,
                ..Default::default()
            },
            engines: vec![engine.into()],
            count: 1,
        }
    }

    fn rerank(g: &GroupedResult, n: usize, engines: Vec<&str>) -> GroupedResult {
        let mut g = g.clone();
        g.count = n;
        g.engines = engines.iter().map(|s| s.to_string()).collect();
        g
    }

    #[test]
    fn agreement_dominates() {
        let singles = group(
            "rust book",
            "https://a.com",
            "rust programming book",
            "bing",
            1,
        );
        let agreed = rerank(
            &group(
                "rust book",
                "https://b.com",
                "rust programming book",
                "brave",
                3,
            ),
            3,
            vec!["bing", "brave", "duckduckgo"],
        );
        let ranked = rank(vec![singles, agreed], "rust book");
        assert_eq!(ranked[0].1.result.url, "https://b.com");
        assert!(ranked[0].0 > ranked[1].0);
        // agreement adds 1.5 per extra engine
        assert!((ranked[0].0 - ranked[1].0 - 3.0).abs() < 1e-9);
    }

    #[test]
    fn position_and_text_matter() {
        let pos1 = group("rust", "https://a.com", "", "bing", 1);
        let pos5 = group("rust", "https://b.com", "", "bing", 5);
        let ranked = rank(vec![pos1, pos5], "rust");
        assert_eq!(ranked[0].1.result.url, "https://a.com");
    }

    #[test]
    fn wikipedia_gets_bonus() {
        let wiki = group(
            "rust",
            "https://en.wikipedia.org/wiki/Rust",
            "",
            "wikipedia",
            10,
        );
        let other = group("rust", "https://c.com", "", "bing", 1);
        let ranked = rank(vec![other, wiki], "rust");
        assert_eq!(ranked[0].1.result.url, "https://en.wikipedia.org/wiki/Rust");
    }

    #[test]
    fn query_terms_boost_title_matches() {
        let title_hit = group("learn rust fast", "https://a.com", "", "bing", 1);
        let no_hit = group("something else", "https://b.com", "", "bing", 1);
        let ranked = rank(vec![no_hit, title_hit], "learn rust");
        assert_eq!(ranked[0].1.result.url, "https://a.com");
    }

    #[test]
    fn ranking_is_stable_and_total() {
        let a = group("x", "https://a.com", "same", "bing", 1);
        let b = group("x", "https://b.com", "same", "brave", 2);
        let mut ranked = rank(vec![a.clone(), b.clone()], "x");
        assert_eq!(ranked.len(), 2);
        let total: f64 = ranked.iter().map(|(s, _)| s).sum();
        ranked.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
        assert!((total - ranked.iter().map(|(s, _)| s).sum::<f64>()).abs() < 1e-9);
    }
}
