//! Temp: probe which wreq-util profiles get real results from Google.

use metasearch::client::{HttpClient, Profile};

fn main() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        for profile in [
            Profile::Chrome,
            Profile::Chrome149,
            Profile::Chrome140,
            Profile::Chrome131,
            Profile::Firefox,
            Profile::Firefox148,
            Profile::Firefox139,
            Profile::Safari,
            Profile::Safari26,
            Profile::Edge,
        ] {
            let client = match HttpClient::builder().profile(profile).build() {
                Ok(c) => c,
                Err(e) => {
                    println!("{profile:?}: build error {e}");
                    continue;
                }
            };
            let url = metasearch::parse::with_query(
                "https://www.google.com/search",
                [
                    ("q", "rust programming"),
                    ("num", "10"),
                    ("hl", "en"),
                    ("lr", "lang_en"),
                    ("ie", "utf8"),
                    ("oe", "utf8"),
                    ("filter", "0"),
                    ("gbv", "1"),
                ],
            );
            match client.get(&url).await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.bytes().await.unwrap_or_default();
                    let text = String::from_utf8_lossy(&body);
                    let h3 = text.matches("<h3").count();
                    let blocked = text.contains("enablejs") || text.contains("/sorry/");
                    println!(
                        "{profile:?}: status={status} h3={h3} blocked={blocked} len={}",
                        body.len()
                    );
                }
                Err(e) => println!("{profile:?}: error {e}"),
            }
        }
    });
}
