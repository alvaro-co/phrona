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
                score += 2.0;
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
