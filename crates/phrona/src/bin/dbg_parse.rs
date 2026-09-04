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
    try_parse!("brave", "brave_web.html", brave::parse_brave);
    try_parse!("mojeek", "mojeek_web.html", mojeek::parse_mojeek);
    try_parse!("google", "google_web.html", google::parse_google);
    try_parse!("yahoo", "yahoo_web.html", yahoo::parse_yahoo);
    try_parse!("yandex", "yandex_web.html", yandex::parse_yandex);
    try_parse!(
        "startpage",
        "startpage_web.html",
        startpage::parse_startpage
    );
    try_parse!(
        "marginalia",
        "marginalia_web.html",
        marginalia::parse_marginalia
    );
    try_parse!("qwant", "qwant_web.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        qwant::parse_qwant(&v, e)
    });
    try_parse!("grokipedia", "grokipedia.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        grokipedia::parse_grokipedia(&v, e)
    });
    try_parse!(
        "wikipedia",
        "wikipedia_opensearch.json",
        |j: &str, e: &str| {
            let v: serde_json::Value = serde_json::from_str(j).unwrap();
            match wikipedia::parse_opensearch(&v) {
                Some((title, url)) => vec![phrona::models::RawResult {
                    title,
                    url,
                    engine: e.to_string(),
                    position: 1,
                    ..Default::default()
                }],
                None => Vec::new(),
            }
        }
    );
    try_parse!("duckduckgo_images", "ddg_images.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        duckduckgo_images::parse_ddg_images(&v, e)
    });
    try_parse!("duckduckgo_news", "ddg_news.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        duckduckgo_news::parse_ddg_news(&v, e)
    });
    try_parse!("duckduckgo_videos", "ddg_videos.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        duckduckgo_videos::parse_ddg_videos(&v, e)
    });
    try_parse!(
        "bing_images",
        "bing_images.html",
        bing_images::parse_bing_images
    );
    try_parse!("bing_news", "bing_news.html", bing_news::parse_bing_news);
    try_parse!(
        "bing_videos",
        "bing_videos.html",
        bing_videos::parse_bing_videos
    );
    try_parse!(
        "brave_images",
        "brave_images.html",
        brave_images::parse_brave_images
    );
    try_parse!(
        "brave_news",
        "brave_news.html",
        brave_news::parse_brave_news
    );
    try_parse!(
        "brave_videos",
        "brave_videos.html",
        brave_videos::parse_brave_videos
    );
    try_parse!(
        "google_images",
        "google_images.json",
        google_images::parse_google_images
    );
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
        "yahoo_news",
        "yahoo_news.html",
        yahoo_news::parse_yahoo_news
    );
    try_parse!("github", "github_code.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        github::parse_github(&v, e)
    });
    try_parse!("arxiv", "arxiv_papers.xml", arxiv::parse_arxiv);
    try_parse!("archive_org", "archive_org.json", |j: &str, e| {
        let v: serde_json::Value = serde_json::from_str(j).unwrap();
        archive_org::parse_archive_org(&v, e)
    });
    try_parse!("annas_archive", "annas_archive.html", |body, name| {
        annas_archive::parse_annas(body, name, "annas-archive.gd")
    });
    eprintln!("unknown engine: {name}");
    std::process::exit(1);
}
