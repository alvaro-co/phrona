use async_trait::async_trait;

use crate::engine::{Engine, EngineContext};
use crate::engines::util;
use crate::engines::util::bing_time_minutes;
use crate::error::{Error, Result};
use crate::models::{Category, RawResult};
use crate::parse;

/// Bing news (infinite-scroll AJAX endpoint).
pub struct BingNews;

#[async_trait]
impl Engine for BingNews {
    fn name(&self) -> &'static str {
        "bing_news"
    }

    fn category(&self) -> Category {
        Category::News
    }

    async fn search(&self, ctx: &EngineContext<'_>) -> Result<Vec<RawResult>> {
        let opts = ctx.opts;
        let (lang, country) = opts.lang_country();
        let mut params: Vec<(&str, String)> = vec![
            ("q", opts.query.clone()),
            ("InfiniteScroll", "1".into()),
            ("first", (opts.page as usize * 10 + 1).to_string()),
            ("SFX", opts.page.to_string()),
            ("cc", country.clone()),
            ("setlang", lang.clone()),
        ];
        if let Some(t) = &opts.time_range {
            params.push(("qft", format!("filterui:age-lt{}", bing_time_minutes(t))));
        }
        let url = parse::with_query("https://www.bing.com/news/infinitescrollajax", params);
        let resp = ctx.client.get(&url).await?;
        util::check_response(self.name(), &resp, util::MediaType::Html)?;
        let body = resp.bytes().await.map_err(Error::from)?;
        let text = String::from_utf8_lossy(&body);
        Ok(parse_bing_news(&text, self.name()))
    }
}

pub fn parse_bing_news(html: &str, engine: &str) -> Vec<RawResult> {
    let doc = parse::parse_html(html);
    let mut out = Vec::new();
    let sel = scraper::Selector::parse("div.newsitem").unwrap();
    let mut pos = 0u32;
    for node in doc.select(&sel) {
        let title = node.value().attr("data-title").unwrap_or("");
        let url = node.value().attr("url").unwrap_or("");
        if title.is_empty() || url.is_empty() {
            continue;
        }
        let published = node
            .select(&scraper::Selector::parse("span[aria-label]").unwrap())
            .next()
            .and_then(|s| s.value().attr("aria-label"))
            .and_then(normalize_date)
            .or_else(|| {
                node.select(&scraper::Selector::parse("span[aria-label]").unwrap())
                    .next()
                    .and_then(|s| s.value().attr("aria-label"))
                    .map(|d| d.to_string())
            });
        let image = node
            .select(&scraper::Selector::parse("a.image img[src]").unwrap())
            .next()
            .map(|i| i.value().attr("src").unwrap_or("").to_string())
            .filter(|s| !s.is_empty());
        pos += 1;
        out.push(RawResult {
            title: title.to_string(),
            url: url.to_string(),
            description: parse::select_text(&node, "div.snippet").unwrap_or_default(),
            published: published.filter(|d| !d.is_empty()),
            source: node.value().attr("data-author").unwrap_or("").to_string(),
            image_url: image.unwrap_or_default(),
            engine: engine.to_string(),
            position: pos,
            ..Default::default()
        });
    }
    out
}

/// Parse absolute ("%d.%m.%Y", "%m/%d/%Y") and relative ("N days ago",
/// multi-language) date strings into ISO-8601.
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
    let (dur, _) = match unit_name {
        "minute" => (chrono::Duration::minutes(num), num),
        "hour" => (chrono::Duration::hours(num), num),
        "day" => (chrono::Duration::days(num), num),
        "week" => (chrono::Duration::weeks(num), num),
        "month" => (chrono::Duration::days(num * 30), num),
        _ => (chrono::Duration::days(num * 365), num),
    };
    let d = now - dur;
    Some(d.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fixture() {
        let html = include_str!("../../tests/fixtures/bing_news.html");
        if !crate::engines::util::fixture_parses("bing_news.html") {
            return;
        }
        let results = parse_bing_news(html, "bing_news");
        assert!(!results.is_empty());
    }

    #[test]
    fn dates() {
        assert!(
            normalize_date("05.08.2026")
                .unwrap()
                .starts_with("2026-08-05")
        );
        assert!(
            normalize_date("08/05/2026")
                .unwrap()
                .starts_with("2026-08-05")
        );
        assert!(normalize_date("3 days ago").is_some());
        assert!(normalize_date("2 Stunden").is_some());
        assert!(normalize_date("5 dias").is_some());
    }
}
