//! Debug tool: parse a fixture with the named parser and print results.
//! Usage: `cargo run -p phrona --bin dbg_parse -- <engine>`

use phrona::engines::*;

fn main() {
    let Some(name) = std::env::args().nth(1) else {
        eprintln!("usage: dbg_parse <engine>");
        std::process::exit(1);
    };
    let dir = "crates/phrona/tests/fixtures";
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
                std::process::exit(0);
            }
        };
    }
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
    try_parse!("grokipedia", "grokipedia.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        grokipedia::parse_grokipedia(&v, e)
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
    eprintln!("unknown engine: {name}");
    std::process::exit(1);
}
