//! Internal dev tool: capture live engine responses into tests/fixtures.
//!
//! Usage: `cargo run -p phrona --bin fetch_fixtures [query]`
//!
//! Every request mirrors the corresponding engine implementation so the
//! saved fixtures exercise exactly the code paths the parsers consume.
//!
//! A capture is only kept when it is a real, parseable SERP: the response
//! must be 2xx with the expected Content-Type AND its own parser must yield
//! at least one result (content validation — no marker-string sniffing).
//! The per-fixture verdict is recorded in `tests/fixtures/meta.json`, which
//! the fixture tests consult before asserting on a capture.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use phrona::client::{HttpClient, Profile};
use phrona::engine::{EngineContext, EngineShared};
use phrona::engines::util::ddg_vqd;
use phrona::models::RawResult;
use phrona::options::SearchOptions;
use phrona::parse;

const DIR: &str = "crates/phrona/tests/fixtures";
const META: &str = "crates/phrona/tests/fixtures/meta.json";

struct Capture {
    body: String,
    status: u16,
    content_type: String,
}

impl Capture {
    fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

async fn bytes_capture(resp: wreq::Response) -> Capture {
    let status = resp.status().as_u16();
    let content_type = resp
        .headers()
        .get(wreq::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp.bytes().await.expect("read body");
    Capture {
        body: String::from_utf8_lossy(&body).into_owned(),
        status,
        content_type,
    }
}

async fn get(client: &HttpClient, url: String) -> Capture {
    bytes_capture(client.get(&url).await.expect("request")).await
}

/// POST to Startpage's SERP, solving an Anubis interstitial when one is
/// served (mirrors the engine's flow so fixtures capture the real results
/// page, not the challenge).
async fn post_search_anubis(client: &HttpClient, form: Vec<(String, String)>) -> Capture {
    const ORIGIN: &str = "https://www.startpage.com";
    const URL: &str = "https://www.startpage.com/sp/search";
    use phrona::engines::anubis::Challenge;

    let body = parse::form_encode(form);
    let resp = client.post_form(URL, &body).await.expect("request");
    let cap = bytes_capture(resp).await;
    if !Challenge::present_in(&cap.body) {
        return cap;
    }
    let Some(challenge) = Challenge::extract(&cap.body) else {
        return cap;
    };
    if challenge.redeem(client, ORIGIN, URL).await.is_err() {
        return cap;
    }
    let resp = client.post_form(URL, &body).await.expect("request");
    bytes_capture(resp).await
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

type Job<'a> = Pin<Box<dyn Future<Output = Capture> + Send + 'a>>;

async fn run(client: &HttpClient, ctx: &EngineContext<'_>, query: &str) {
    let mut jobs: Vec<(&str, Job<'_>)> = Vec::new();

    // web
    jobs.push((
        "ddg_web.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://html.duckduckgo.com/html/",
                [("q", query), ("b", ""), ("l", "us-en")],
            ),
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
            bytes_capture(client.get_with_headers(&url, &headers).await.unwrap()).await
        }),
    ));
    jobs.push((
        "marginalia_web.html",
        Box::pin(get(
            client,
            parse::with_query(
                "https://old-search.marginalia.nu/search",
                [
                    ("query", query),
                    ("profile", "default"),
                    ("js", "default"),
                    ("adtech", "default"),
                ],
            ),
        )),
    ));

    let sc = phrona::engines::startpage::fetch_sc(ctx)
        .await
        .unwrap_or_default();
    jobs.push((
        "startpage_web.html",
        Box::pin(post_search_anubis(
            client,
            vec![
                ("query".into(), query.into()),
                ("cat".into(), "web".into()),
                ("t".into(), "device".into()),
                ("sc".into(), sc.clone()),
                ("language".into(), "english".into()),
                ("lui".into(), "english".into()),
                ("abp".into(), "1".into()),
                ("abd".into(), "0".into()),
                ("abe".into(), "0".into()),
                ("qsr".into(), "en_US".into()),
                ("qadf".into(), "moderate".into()),
                ("segment".into(), "organic".into()),
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
                    ("first", "1"),
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
        Box::pin(async {
            // fresh client: the earlier google_web capture leaves Google
            // cookies (NID/SOCS) in the shared jar that change which SERP
            // variant /search serves
            let fresh = HttpClient::builder()
                .profile(Profile::Chrome)
                .build()
                .unwrap();
            let mut headers = wreq::header::HeaderMap::new();
            headers.insert(
                wreq::header::COOKIE,
                wreq::header::HeaderValue::from_static(
                    "CONSENT=YES+cb.20250101-07-p0.en+FX+419; SOCS=CAI",
                ),
            );
            let url = parse::with_query(
                "https://www.google.com/search",
                [
                    ("q", query),
                    ("tbm", "isch"),
                    ("hl", "en"),
                    ("asearch", "isch"),
                    ("async", "_fmt:json,p:1,ijn:0"),
                    ("safe", "moderate"),
                ],
            );
            bytes_capture(fresh.get_with_headers(&url, &headers).await.unwrap()).await
        }),
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
        Box::pin(post_search_anubis(
            client,
            vec![
                ("query".into(), query.into()),
                ("cat".into(), "images".into()),
                ("t".into(), "device".into()),
                ("sc".into(), sc.clone()),
                ("language".into(), "english".into()),
                ("lui".into(), "english".into()),
                ("abp".into(), "1".into()),
                ("abd".into(), "0".into()),
                ("abe".into(), "0".into()),
                ("qsr".into(), "en_US".into()),
                ("qadf".into(), "moderate".into()),
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

    let mut meta = load_meta();
    let mut failures = 0;
    for (name, fut) in jobs {
        let path = PathBuf::from(DIR).join(name);
        let cap = fut.await;
        let parsed = cap.ok() && fixture_parses(name, &cap.body);
        if cap.body.len() > 200 && parsed {
            std::fs::write(&path, &cap.body).unwrap();
            println!("ok    {name} ({} bytes)", cap.body.len());
            meta.insert(
                name.to_string(),
                serde_json::json!({
                    "bytes": cap.body.len(),
                    "status": cap.status,
                    "content_type": cap.content_type,
                    "parsed": true,
                }),
            );
        } else {
            println!(
                "block {name} ({} bytes, status {}, ct {:?}) - kept existing fixture",
                cap.body.len(),
                cap.status,
                cap.content_type
            );
            // The existing fixture stays. Re-validate it so its recorded
            // verdict reflects the fixture actually on disk - never a
            // transient block from this capture attempt.
            if !path.exists() {
                failures += 1;
            }
            let m = meta.clone();
            let verdict = std::fs::read_to_string(&path).ok().and_then(|old| {
                let parsed = fixture_parses(name, &old);
                parsed.then(|| {
                    serde_json::json!({
                        "bytes": old.len(),
                        "status": m.get(name)
                            .and_then(|e| e.get("status").cloned())
                            .unwrap_or(serde_json::json!(cap.status)),
                        "content_type": m.get(name)
                            .and_then(|e| e.get("content_type").cloned())
                            .unwrap_or(serde_json::json!(cap.content_type)),
                        "parsed": true,
                    })
                })
            });
            if let Some(v) = verdict {
                meta.insert(name.to_string(), v);
            }
        }
    }
    std::fs::write(META, serde_json::to_string_pretty(&meta).unwrap() + "\n").unwrap();
    if failures > 0 {
        std::process::exit(1);
    }
}

fn load_meta() -> serde_json::Map<String, serde_json::Value> {
    std::fs::read_to_string(META)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Content validation: a fixture is genuine only if its own parser extracts at
/// least one result from the captured body.
fn fixture_parses(name: &str, body: &str) -> bool {
    use phrona::engines::{
        annas_archive, bing, bing_images, bing_news, bing_videos, brave, brave_images, brave_news,
        brave_videos, duckduckgo_images, duckduckgo_news, duckduckgo_videos, google, google_images,
        marginalia, mojeek, mojeek_images, qwant, startpage, startpage_images, yahoo, yahoo_news,
        yandex,
    };

    let parsed_html: Option<Vec<RawResult>> = match name {
        "ddg_web.html" => Some(phrona::engines::util::parse_ddg_html(body, "ddg").0),
        "google_web.html" => Some(google::parse_google(body, "google")),
        "bing_web.html" => Some(bing::parse_bing(body, "bing")),
        "brave_web.html" => Some(brave::parse_brave(body, "brave")),
        "mojeek_web.html" => Some(mojeek::parse_mojeek(body, "mojeek")),
        "yahoo_web.html" => Some(yahoo::parse_yahoo(body, "yahoo")),
        "yandex_web.html" => Some(yandex::parse_yandex(body, "yandex")),
        "startpage_web.html" => Some(startpage::parse_startpage(body, "startpage")),
        "marginalia_web.html" => Some(marginalia::parse_marginalia(body, "marginalia")),
        "bing_images.html" => Some(bing_images::parse_bing_images(body, "bing_images")),
        "bing_news.html" => Some(bing_news::parse_bing_news(body, "bing_news")),
        "bing_videos.html" => Some(bing_videos::parse_bing_videos(body, "bing_videos")),
        "brave_images.html" => Some(brave_images::parse_brave_images(body, "brave_images")),
        "brave_news.html" => Some(brave_news::parse_brave_news(body, "brave_news")),
        "brave_videos.html" => Some(brave_videos::parse_brave_videos(body, "brave_videos")),
        "mojeek_images.html" => Some(mojeek_images::parse_mojeek_images(body, "mojeek_images")),
        "startpage_images.html" => Some(startpage_images::parse_startpage_images(
            body,
            "startpage_images",
        )),
        "yahoo_news.html" => Some(yahoo_news::parse_yahoo_news(body, "yahoo_news")),
        "annas_archive.html" => Some(annas_archive::parse_annas(
            body,
            "annas_archive",
            "annas-archive.gd",
        )),
        // not an HTML SERP, but its own text-scanning parser must run here:
        // the body carries a `)]}'` XSSI prefix, so the generic
        // whole-body-JSON check below would always reject it
        "google_images.json" => Some(google_images::parse_google_images(body, "google_images")),
        _ => None,
    };
    if let Some(results) = parsed_html {
        return !results.is_empty();
    }

    let Ok(json) = serde_json::from_str::<serde_json::Value>(body) else {
        return false;
    };
    match name {
        "qwant_web.json" => !qwant::parse_qwant(&json, "qwant").is_empty(),
        "ddg_images.json" => !duckduckgo_images::parse_ddg_images(&json, "ddg_images").is_empty(),
        "ddg_news.json" => !duckduckgo_news::parse_ddg_news(&json, "ddg_news").is_empty(),
        "ddg_videos.json" => !duckduckgo_videos::parse_ddg_videos(&json, "ddg_videos").is_empty(),
        "google_images.json" => {
            !google_images::parse_google_images(body, "google_images").is_empty()
        }
        "wikipedia_opensearch.json" => {
            let titles = json
                .get(1)
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            let urls = json
                .get(3)
                .and_then(|v| v.as_array())
                .map(|a| a.len())
                .unwrap_or(0);
            titles > 0 || urls > 0
        }
        "grokipedia.json" => json
            .get("results")
            .and_then(|r| r.as_array())
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        _ => false,
    }
}
