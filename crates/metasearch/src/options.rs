use std::time::Duration;

use crate::client::Profile;
use crate::models::{Category, SafeSearch, TimeRange};

/// Search parameters shared by every engine.
#[derive(Debug, Clone)]
pub struct SearchOptions {
    pub query: String,
    pub category: Category,
    /// Restrict to these engines. Empty means "all enabled engines for the category".
    pub engines: Vec<String>,
    pub page: u32,
    pub max_results: usize,
    pub safesearch: SafeSearch,
    pub region: Option<String>,
    pub language: Option<String>,
    pub time_range: Option<TimeRange>,
    /// Engine-specific free-form filter string (e.g. DDG image filters
    /// "size:Large,color:red"). Passed through to engines that support it.
    pub filters: Option<String>,
    pub profile: Profile,
    pub timeout: Duration,
    pub proxies: Vec<String>,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            query: String::new(),
            category: Category::Web,
            engines: Vec::new(),
            page: 1,
            max_results: 20,
            safesearch: SafeSearch::Moderate,
            region: None,
            language: None,
            time_range: None,
            filters: None,
            profile: Profile::Chrome,
            timeout: Duration::from_secs(10),
            proxies: Vec::new(),
        }
    }
}

impl SearchOptions {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            ..Default::default()
        }
    }

    /// "us-en" style region; returns "en" language part and "us" country part.
    pub fn lang_country(&self) -> (String, String) {
        let region = self
            .region
            .as_deref()
            .and_then(|r| r.split('-').next())
            .filter(|l| !l.is_empty() && l.len() <= 10);
        let lang = self
            .language
            .as_deref()
            .or(region)
            .unwrap_or("en")
            .to_string();
        let country = self
            .region
            .as_deref()
            .and_then(|r| r.split('-').nth(1))
            .filter(|c| !c.is_empty() && c.len() <= 10)
            .unwrap_or("us")
            .to_string();
        (lang, country)
    }

    pub fn region_param(&self) -> String {
        self.region.clone().unwrap_or_else(|| {
            let (lang, country) = self.lang_country();
            format!("{lang}-{country}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enums_parse_from_str() {
        for (s, expect) in [
            ("web", Category::Web),
            ("images", Category::Images),
            ("news", Category::News),
            ("videos", Category::Videos),
            ("books", Category::Books),
        ] {
            assert_eq!(s.parse::<Category>().unwrap(), expect);
        }
        assert!("nope".parse::<Category>().is_err());

        for (s, expect) in [
            ("off", SafeSearch::Off),
            ("moderate", SafeSearch::Moderate),
            ("strict", SafeSearch::Strict),
        ] {
            assert_eq!(s.parse::<SafeSearch>().unwrap(), expect);
        }
        assert!("x".parse::<SafeSearch>().is_err());

        for (s, expect) in [
            ("day", TimeRange::Day),
            ("week", TimeRange::Week),
            ("month", TimeRange::Month),
            ("year", TimeRange::Year),
        ] {
            assert_eq!(s.parse::<TimeRange>().unwrap(), expect);
        }
        assert!("yesterday".parse::<TimeRange>().is_err());
    }

    #[test]
    fn defaults_are_sane() {
        let o = SearchOptions::default();
        assert_eq!(o.category, Category::Web);
        assert_eq!(o.page, 1);
        assert_eq!(o.max_results, 20);
        assert_eq!(o.safesearch, SafeSearch::Moderate);
        assert_eq!(o.engines, Vec::<String>::new());
        assert!(o.proxies.is_empty());
    }

    #[test]
    fn new_sets_query_only() {
        let o = SearchOptions::new("hello world");
        assert_eq!(o.query, "hello world");
        assert_eq!(o.category, Category::Web);
    }

    #[test]
    fn lang_country_derivation() {
        let o = SearchOptions {
            region: Some("de-de".into()),
            language: None,
            ..Default::default()
        };
        assert_eq!(o.lang_country(), ("de".into(), "de".into()));
        assert_eq!(o.region_param(), "de-de");

        let o = SearchOptions::default();
        assert_eq!(o.lang_country(), ("en".into(), "us".into()));
        assert_eq!(o.region_param(), "en-us");

        let o = SearchOptions {
            language: Some("fr".into()),
            region: Some("fr-fr".into()),
            ..Default::default()
        };
        assert_eq!(o.lang_country(), ("fr".into(), "fr".into()));
    }
}
