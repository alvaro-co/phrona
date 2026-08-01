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
