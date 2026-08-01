//! MetaSearchRS - a high-performance metasearch engine library.

pub mod client;
pub mod error;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version() -> &'static str {
    VERSION
}

#[cfg(test)]
mod tests {
    use crate::client::{HttpClient, Profile};

    #[tokio::test]
    async fn smoke_request() {
        let client = HttpClient::builder().profile(Profile::Chrome).build().unwrap();
        let resp = client.get("https://html.duckduckgo.com/").await.unwrap();
        assert_eq!(resp.status(), 200);
    }
}
