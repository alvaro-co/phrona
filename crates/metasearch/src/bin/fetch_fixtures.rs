//! Internal dev tool: capture live engine responses into tests/fixtures.
//!
//! Usage: cargo run -p metasearch --bin fetch_fixtures [query]
//!
//! Every request mirrors the corresponding engine implementation so the
//! saved fixtures exercise exactly the code paths the parsers consume.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use metasearch::client::{HttpClient, Profile};
use metasearch::engine::{EngineContext, EngineShared};
use metasearch::engines::util::ddg_vqd;
use metasearch::options::SearchOptions;
use metasearch::parse;

const DIR: &str = "crates/metasearch/tests/fixtures";

async fn bytes_text(resp: wreq::Response) -> String {
    let body = resp.bytes().await.expect("read body");
    String::from_utf8_lossy(&body).into_owned()
}

async fn get(client: &HttpClient, url: String) -> String {
    bytes_text(client.get(&url).await.expect("request")).await
}

async fn post_form(client: &HttpClient, url: String, form: Vec<(String, String)>) -> String {
    let body = parse::form_encode(form);
    let resp = client.post_form(&url, &body).await.expect("request");
    bytes_text(resp).await
}

fn main() {
    let query = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "rust programming".to_string());
    let query = query.as_str();
    let client = HttpClient::builder()
        .profile(Profile::Chrome)
        .build()
        .unwrap();
    let shared = EngineShared::new();
    let opts = SearchOptions::new(query);
    let ctx = EngineContext {
        client: &client,
        opts: &opts,
        shared: &shared,
    };
    std::fs::create_dir_all(DIR).unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(run(&client, &ctx, query));
}

type Job<'a> = Pin<Box<dyn Future<Output = String> + Send + 'a>>;

async fn run(client: &HttpClient, ctx: &EngineContext<'_>, query: &str) {
    let mut jobs: Vec<(&str, Job<'_>)> = Vec::new();

    // web
    jobs.push((
        "ddg_web.html",
        Box::pin(post_form(
            client,
            "https://html.duckduckgo.com/html/".to_string(),
            vec![
                ("q".into(), query.into()),
                ("b".into(), String::new()),
                ("l".into(), "us-en".into()),
                ("s".into(), "0".into()),
            ],
        )),
    ));
    jobs.push((
        "google_web.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://www.google.com/search",
                [
                    ("q", query),
                    ("num", "20"),
                    ("hl", "en"),
                    ("lr", "lang_en"),
                    ("safe", "active"),
                ],
            ),
        )),
    ));
    jobs.push((
        "bing_web.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://www.bing.com/search",
                [
                    ("q", query),
                    ("count", "20"),
                    ("first", "1"),
                    ("mkt", "en-US"),
                    ("setlang", "en"),
                ],
            ),
        )),
    ));
    jobs.push((
        "brave_web.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://search.brave.com/search",
                [("q", query), ("source", "web")],
            ),
        )),
    ));
    jobs.push((
        "mojeek_web.html",
        Box::pin(get(
            client,
            parse::with_query("https://www.mojeek.com/search", [("q", query)]),
        )),
    ));
    jobs.push((
        "yahoo_web.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://search.yahoo.com/search;_ylt=A;_ylu=B",
                [("p", query), ("ei", "UTF-8")],
            ),
        )),
    ));
    jobs.push((
        "yandex_web.html",
        Box::pin(async move {
            let mut headers = wreq::header::HeaderMap::new();
            headers.insert(
                wreq::header::COOKIE,
                wreq::header::HeaderValue::from_static(
                    "yp=1716337604.sp.family%3A0#1685406411.szm.1:1920x1080:1920x999",
                ),
            );
            let url = parse::with_query(
                "https://yandex.com/search/site/",
                [
                    ("text", query),
                    ("web", "1"),
                    ("frame", "1"),
                    ("tmpl_version", "releases"),
                    ("searchid", "3131712"),
                    ("lang", "en"),
                ],
            );
            bytes_text(client.get_with_headers(&url, &headers).await.unwrap()).await
        }),
    ));

    let sc = metasearch::engines::startpage::fetch_sc(ctx)
        .await
        .unwrap_or_default();
    jobs.push((
        "startpage_web.html",
        Box::pin(post_form(
            client,
            "https://www.startpage.com/sp/search".to_string(),
            vec![
                ("query".into(), query.into()),
                ("t".into(), "device".into()),
                ("sc".into(), sc.clone()),
                ("qsr".into(), "en_US".into()),
                ("language".into(), "english".into()),
            ],
        )),
    ));
    jobs.push((
        "qwant_web.json",
        Box::pin(get(
            client,
            parse::with_query(
                "https://api.qwant.com/v3/search/web",
                [
                    ("q", query),
                    ("count", "10"),
                    ("locale", "en_US"),
                    ("offset", "0"),
                    ("safesearch", "1"),
                    ("device", "desktop"),
                    ("displayed", "true"),
                    ("llm", "true"),
                    ("tgp", "abcdefg"),
                ],
            ),
        )),
    ));
    jobs.push((
        "wikipedia_opensearch.json",
        Box::pin(get(
            client,
            parse::with_query(
                "https://en.wikipedia.org/w/api.php",
                [
                    ("action", "opensearch"),
                    ("profile", "fuzzy"),
                    ("limit", "10"),
                    ("search", query),
                ],
            ),
        )),
    ));
    jobs.push((
        "grokipedia.json",
        Box::pin(get(
            client,
            parse::with_query(
                "https://grokipedia.com/api/typeahead",
                [("query", query), ("limit", "1")],
            ),
        )),
    ));

    // images
    let vqd = ddg_vqd(ctx, query).await.unwrap_or_default();
    jobs.push((
        "ddg_images.json",
        Box::pin(get(
            client,
            parse::with_query(
                "https://duckduckgo.com/i.js",
                [
                    ("o", "json"),
                    ("q", query),
                    ("l", "us-en"),
                    ("vqd", &vqd),
                    ("p", "1"),
                    ("ct", "AT"),
                ],
            ),
        )),
    ));
    jobs.push((
        "ddg_news.json",
        Box::pin(get(
            client,
            parse::with_query(
                "https://duckduckgo.com/news.js",
                [
                    ("l", "us-en"),
                    ("o", "json"),
                    ("noamp", "1"),
                    ("q", query),
                    ("vqd", &vqd),
                    ("p", "-1"),
                ],
            ),
        )),
    ));
    jobs.push((
        "ddg_videos.json",
        Box::pin(get(
            client,
            parse::with_query(
                "https://duckduckgo.com/v.js",
                [
                    ("l", "us-en"),
                    ("o", "json"),
                    ("noamp", "1"),
                    ("q", query),
                    ("vqd", &vqd),
                    ("p", "-1"),
                ],
            ),
        )),
    ));
    jobs.push((
        "bing_images.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://www.bing.com/images/async",
                [
                    ("q", query),
                    ("first", "0"),
                    ("count", "35"),
                    ("mkt", "en-US"),
                ],
            ),
        )),
    ));
    jobs.push((
        "bing_news.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://www.bing.com/news/infinitescrollajax",
                [
                    ("q", query),
                    ("InfiniteScroll", "1"),
                    ("first", "11"),
                    ("SFX", "1"),
                    ("cc", "US"),
                    ("setlang", "en"),
                ],
            ),
        )),
    ));
    jobs.push((
        "bing_videos.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://www.bing.com/videos/asyncv2",
                [
                    ("q", query),
                    ("async", "content"),
                    ("first", "1"),
                    ("count", "35"),
                    ("mkt", "en-US"),
                ],
            ),
        )),
    ));
    jobs.push((
        "brave_images.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://search.brave.com/images",
                [("q", query), ("source", "web")],
            ),
        )),
    ));
    jobs.push((
        "brave_news.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://search.brave.com/news",
                [("q", query), ("source", "web")],
            ),
        )),
    ));
    jobs.push((
        "brave_videos.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://search.brave.com/videos",
                [("q", query), ("source", "web")],
            ),
        )),
    ));
    jobs.push((
        "google_images.json",
        Box::pin(get(
            client,
            parse::with_query(
                "https://www.google.com/search",
                [
                    ("tbm", "isch"),
                    ("q", query),
                    ("num", "20"),
                    ("async", "_fmt:json,p:1,ijn:0"),
                ],
            ),
        )),
    ));
    jobs.push((
        "mojeek_images.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://www.mojeek.com/search",
                [("q", query), ("fmt", "images")],
            ),
        )),
    ));
    jobs.push((
        "startpage_images.html",
        Box::pin(post_form(
            client,
            "https://www.startpage.com/sp/search".to_string(),
            vec![
                ("query".into(), query.into()),
                ("cat".into(), "images".into()),
                ("t".into(), "device".into()),
                ("sc".into(), sc.clone()),
                ("qsr".into(), "en_US".into()),
                ("segment".into(), "startpage.udog".into()),
            ],
        )),
    ));
    jobs.push((
        "yahoo_news.html",
        Box::pin(get(
            client,
            parse::with_query("https://news.search.yahoo.com/search", [("p", query)]),
        )),
    ));
    jobs.push((
        "annas_archive.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://annas-archive.gd/search",
                [("q", query), ("page", "1")],
            ),
        )),
    ));

    let mut failures = 0;
    for (name, fut) in jobs {
        let path = PathBuf::from(DIR).join(name);
        let body = fut.await;
        if body.len() > 200 && !metasearch::engines::util::is_block_page(&body) {
            std::fs::write(&path, &body).unwrap();
            println!("ok    {name} ({} bytes)", body.len());
        } else {
            println!(
                "block {name} ({} bytes) - kept existing fixture",
                body.len()
            );
            if !path.exists() {
                failures += 1;
            }
        }
    }
    if failures > 0 {
        std::process::exit(1);
    }
}
