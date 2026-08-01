//! Temporary debug tool: parse a fixture with the named parser and print results.

use metasearch::engines::*;

fn main() {
    let name = std::env::args().nth(1).unwrap();
    let query = std::env::args().nth(2).unwrap_or_default();
    let dir = "crates/metasearch/tests/fixtures";
    macro_rules! try_parse {
        ($n:expr, $fixture:expr, $f:expr) => {
            if name == $n {
                let data = std::fs::read_to_string(format!("{dir}/{}", $fixture)).unwrap();
                let results = $f(&data, $n);
                println!("{} -> {} results", $n, results.len());
                for r in results.iter().take(3) {
                    println!("  title={:?} url={:?}", r.title, r.url);
                    if !r.description.is_empty() {
                        println!("  desc={:?}", &r.description[..r.description.len().min(80)]);
                    }
                    if !r.image_url.is_empty() {
                        println!("  img={:?}", r.image_url);
                    }
                    if !r.thumbnail_url.is_empty() {
                        println!("  thumb={:?}", r.thumbnail_url);
                    }
                }
            }
        };
    }
    let _ = query;
    try_parse!("duckduckgo", "ddg_web.html", |d: &str, e| {
        util::parse_ddg_html(d, e).0
    });
    try_parse!("bing", "bing_web.html", bing::parse_bing);
    try_parse!(
        "bing_videos",
        "bing_videos.html",
        bing_videos::parse_bing_videos
    );
    try_parse!(
        "google_images",
        "google_images.json",
        google_images::parse_google_images
    );
    try_parse!("grokipedia", "grokipedia.json", |j: &str, e: &str| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        let Some(item) = v
            .get("results")
            .and_then(|r| r.as_array())
            .and_then(|a| a.first())
        else {
            return Vec::new();
        };
        let Some(items) = item.get("items").and_then(|i| i.as_array()) else {
            return Vec::new();
        };
        let Some(first) = items.first() else {
            return Vec::new();
        };
        let title = first
            .get("title")
            .and_then(|t| t.as_str())
            .unwrap_or("")
            .trim_matches('_');
        let snippet = first.get("snippet").and_then(|s| s.as_str()).unwrap_or("");
        let description = snippet
            .split_once("\n\n")
            .map(|(a, _)| a)
            .unwrap_or(snippet);
        let slug = first.get("slug").and_then(|s| s.as_str()).unwrap_or("");
        vec![metasearch::models::RawResult {
            title: title.to_string(),
            url: format!("https://grokipedia.com/page/{slug}"),
            description: description.to_string(),
            engine: e.to_string(),
            position: 1,
            ..Default::default()
        }]
    });
    try_parse!("mojeek", "mojeek_web.html", mojeek::parse_mojeek);
    try_parse!("google", "google_web.html", google::parse_google);
    try_parse!("qwant", "qwant_web.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        qwant::parse_qwant(&v, e)
    });
    try_parse!(
        "mojeek_images",
        "mojeek_images.html",
        mojeek_images::parse_mojeek_images
    );
    try_parse!(
        "startpage_images",
        "startpage_images.html",
        startpage_images::parse_startpage_images
    );
    try_parse!(
        "brave_images",
        "brave_images.html",
        brave_images::parse_brave_images
    );
    try_parse!("yandex", "yandex_web.html", yandex::parse_yandex);
    try_parse!(
        "yahoo_news",
        "yahoo_news.html",
        yahoo_news::parse_yahoo_news
    );
    try_parse!(
        "annas_archive",
        "annas_archive.html",
        annas_archive::parse_annas
    );
}
