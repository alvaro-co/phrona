//! Phrona - a high-performance metasearch engine library.

pub mod client;
pub mod config;
pub mod dedup;
pub mod engine;
pub mod engines;
pub mod error;
pub mod extract;
pub mod models;
pub mod options;
pub mod parse;
pub mod rank;
pub mod search;

pub use client::{HttpClient, HttpClientBuilder, Profile};
pub use config::{ConfigError, PhronaConfig};
pub use error::{Error, Result};
pub use extract::{ExtractedPage, extract, extract_from_html, extract_many, is_safe_ip};
pub use models::*;
pub use options::SearchOptions;
pub use search::{
    EngineObserver, NoopEngineObserver, SearchClient, available_engines, search, search_sync,
};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

pub use engines::suggest::{SuggestSource, suggest, suggest_all};

pub use engine::{Engine, EngineContext, EngineShared};

#[cfg(test)]
mod tests {
    use crate::client::{HttpClient, Profile};

    /// Live-network smoke test; excluded from `cargo test` runs (needs the
    /// internet and an unblocked IP).
    #[tokio::test]
    #[ignore]
    async fn smoke_request() {
        let client = HttpClient::builder()
            .profile(Profile::Chrome)
            .build()
            .unwrap();
        let resp = client.get("https://html.duckduckgo.com/").await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}
