mod args;
mod output;

use anyhow::Result;
use clap::Parser;
use serde_json::json;

use phrona::SuggestSource;
use phrona::models::ResultItem;

use args::{Cli, Command, TestArgs};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = phrona::SearchClient::with_options(
        cli.profile,
        Some(std::time::Duration::from_secs(cli.timeout)),
        (!cli.proxy.is_empty()).then(|| cli.proxy.clone()),
    )?;

    match &cli.command {
        Command::Search(args) => {
            let mut opts = cli.base_options(&args.query);
            opts.category = args.category;
            opts.engines = split_engines(args.engines.as_deref());
            opts.max_results = args.max_results.clamp(1, 100);
            opts.safesearch = args.safesearch;
            opts.region = args.region.clone();
            opts.language = args.language.clone();
            opts.time_range = args.time_range;
            opts.filters = args.filters.clone();
            opts.page = args.page.max(1);
            let resp = client.search(opts).await?;
            if cli.json {
                print_json(&resp);
            } else {
                output::print_response(&resp);
            }
        }
        Command::Suggest(args) => {
            let region = &args.region;
            let sources = split_sources(args.source.as_deref())?;
            if sources.is_empty() {
                let all = phrona::suggest_all(client.http(), &args.query, region).await;
                if cli.json {
                    let map: serde_json::Map<String, _> = all
                        .into_iter()
                        .map(|(s, list)| (s.name().to_string(), json!(list)))
                        .collect();
                    println!("{}", json!(map));
                } else {
                    for (s, list) in all {
                        println!("{}: {}", s.name(), list.join(" | "));
                    }
                }
            } else {
                for s in sources {
                    let list = phrona::suggest(client.http(), s, &args.query, region).await?;
                    if cli.json {
                        println!("{}", json!({"source": s.name(), "suggestions": list}));
                    } else {
                        println!("{}: {}", s.name(), list.join(" | "));
                    }
                }
            }
        }
        Command::Extract(args) => {
            let results = phrona::extract_many(
                client.http(),
                &args.urls,
                args.max_chars,
                args.query.as_deref(),
            )
            .await;
            let mut failed = false;
            for (url, result) in args.urls.iter().zip(results) {
                match result {
                    Ok(page) => {
                        if cli.json {
                            println!("{}", serde_json::to_string_pretty(&page)?);
                        } else {
                            if args.urls.len() > 1 {
                                println!("== {url}");
                            }
                            println!("title: {}\n", page.title);
                            if !page.description.is_empty() {
                                println!("description: {}\n", page.description);
                            }
                            println!("{}", page.text);
                            if !page.images.is_empty() {
                                println!("\nimages: {}", page.images.join(" | "));
                            }
                            println!();
                        }
                    }
                    Err(e) => {
                        failed = true;
                        eprintln!("{url}: {e}");
                    }
                }
            }
            if failed {
                std::process::exit(1);
            }
        }
        Command::Ground(args) => {
            let mut opts = cli.base_options(&args.query);
            opts.max_results = args.max_results.clamp(1, 100);
            opts.engines = split_engines(args.engines.as_deref());
            opts.category = args.category;
            opts.region = args.region.clone();
            opts.language = args.language.clone();
            opts.time_range = args.time_range;
            opts.safesearch = args.safesearch;
            opts.filters = args.filters.clone();
            opts.page = args.page.max(1);
            let resp = client.search(opts).await?;
            if cli.json {
                print_json(&resp);
            } else {
                output::print_grounded(&args.query, &resp, args.max_results);
            }
        }
        Command::Engines(args) => {
            let cats: Vec<phrona::Category> = match args.category {
                Some(c) => vec![c],
                None => phrona::Category::ALL.to_vec(),
            };
            if cli.json {
                let mut map = serde_json::Map::new();
                for c in cats {
                    let names: Vec<String> = phrona::available_engines(c)
                        .iter()
                        .map(|e| e.name.clone())
                        .collect();
                    map.insert(c.as_str().to_string(), json!(names));
                }
                println!("{}", json!(map));
            } else {
                for c in cats {
                    output::print_engines_table(c);
                }
            }
        }
        Command::Test(args) => {
            run_test(&cli, &client, args).await?;
        }
        Command::Serve(args) => {
            run_serve(args).await?;
        }
        Command::Mcp => {
            init_tracing();
            phrona_mcp::run_stdio().await?;
        }
        Command::Completions(args) => {
            args::print_completions(&args.shell)?;
        }
    }
    Ok(())
}

fn init_tracing() {
    let filter = std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new(filter))
        .init();
}

/// Full server: REST API (axum) plus MCP-over-TCP, in one process.
async fn run_serve(args: &args::ServeArgs) -> anyhow::Result<()> {
    init_tracing();
    let api_key = args.api_key.clone().or_else(|| {
        std::env::var("PHRONA_API_KEY")
            .ok()
            .filter(|k| !k.is_empty())
    });

    let rest_addr = match &args.addr {
        Some(a) => a.parse()?,
        None => std::env::var("PHRONA_ADDR")
            .ok()
            .map(|a| a.parse())
            .transpose()?
            .unwrap_or_else(phrona_api::default_addr),
    };
    let mcp_addr = args.mcp_addr.clone();

    let rest_fut = async {
        if !args.no_rest {
            phrona_api::serve(rest_addr, api_key.clone()).await?;
        }
        anyhow::Ok(())
    };
    let mcp_fut = async {
        if !args.no_mcp {
            let listener = phrona_mcp::tcp_listener(&mcp_addr).await?;
            tracing::info!("phrona-mcp listening on tcp://{mcp_addr} (newline-delimited JSON-RPC)");
            phrona_mcp::serve_tcp(listener).await?;
        }
        anyhow::Ok(())
    };

    // Join both listeners: a disabled listener resolves immediately and the
    // other keeps running (a tokio::select! here would exit when the
    // disabled one completes and kill the server).
    futures::future::try_join(rest_fut, mcp_fut).await?;
    Ok(())
}

fn split_engines(s: Option<&str>) -> Vec<String> {
    s.map(|s| s.split(',').map(|e| e.trim().to_string()).collect())
        .unwrap_or_default()
}

fn split_sources(s: Option<&str>) -> anyhow::Result<Vec<SuggestSource>> {
    let s = s.unwrap_or_default();
    let mut out = Vec::new();
    for n in s.split(',').map(str::trim).filter(|n| !n.is_empty()) {
        match SuggestSource::from_name(n) {
            Some(src) => out.push(src),
            None => anyhow::bail!(
                "unknown suggest source '{n}', expected one of: {}",
                SuggestSource::ALL
                    .iter()
                    .map(|s| s.name())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
    Ok(out)
}

fn print_json(resp: &phrona::SearchResponse) {
    println!("{}", search_json(resp));
}

/// Machine-readable JSON for a search response. Pure (no stdout), so the
/// exact payload is unit-testable.
fn search_json(resp: &phrona::SearchResponse) -> serde_json::Value {
    let results: Vec<_> = resp
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let pos = (i + 1) as f64;
            let score = (1.0 - (pos - 1.0) * 0.05).max(0.05);
            match r {
                ResultItem::Web(w) => json!({
                    "type": "web", "title": w.title, "url": w.url,
                    "description": w.description, "score": w.score, "position": pos,
                    "engines": w.engines,
                }),
                ResultItem::Image(im) => json!({
                    "type": "image", "title": im.title, "url": im.url,
                    "image_url": im.image_url, "thumbnail_url": im.thumbnail_url,
                    "width": im.width, "height": im.height, "score": score, "position": pos,
                    "engines": im.engines,
                }),
                ResultItem::News(n) => json!({
                    "type": "news", "title": n.title, "url": n.url,
                    "description": n.description, "published": n.published,
                    "source": n.source, "score": score, "position": pos, "engines": n.engines,
                }),
                ResultItem::Video(v) => json!({
                    "type": "video", "title": v.title, "url": v.url,
                    "description": v.description, "thumbnail_url": v.thumbnail_url,
                    "duration": v.duration, "views": v.views, "uploader": v.uploader,
                    "score": score, "position": pos, "engines": v.engines,
                }),
                ResultItem::Book(b) => json!({
                    "type": "book", "title": b.title, "url": b.url,
                    "author": b.author, "publisher": b.publisher, "info": b.info,
                    "score": score, "position": pos, "engines": b.engines,
                }),
            }
        })
        .collect();
    json!({
        "query": resp.query,
        "category": resp.category.as_str(),
        "page": resp.page,
        "total": resp.total,
        "results": results,
        "suggestions": resp.suggestions,
        "answer": resp.answer,
        "engines": resp.engines,
        "elapsed_ms": resp.elapsed_ms,
    })
}

async fn run_test(cli: &Cli, client: &phrona::SearchClient, args: &TestArgs) -> Result<()> {
    let cats: Vec<phrona::Category> = match args.category {
        Some(c) => vec![c],
        None => phrona::Category::ALL.to_vec(),
    };
    let mut reports = Vec::new();
    let mut any_success = false;
    for cat in cats {
        let mut opts = cli.base_options(&args.query);
        opts.category = cat;
        opts.max_results = args.max_results.clamp(1, 10);
        match client.search(opts).await {
            Ok(resp) => {
                any_success = true;
                reports.push((cat, resp));
            }
            Err(e) => {
                reports.push((
                    cat,
                    phrona::SearchResponse {
                        query: args.query.clone(),
                        category: cat,
                        page: 1,
                        total: 0,
                        results: Vec::new(),
                        suggestions: Vec::new(),
                        answer: None,
                        engines: Vec::new(),
                        elapsed_ms: 0,
                    },
                ));
                eprintln!("category {}: {e}", cat.as_str());
            }
        }
    }
    if cli.json {
        let out: Vec<_> = reports
            .iter()
            .map(|(cat, r)| {
                json!({
                    "category": cat.as_str(),
                    "total": r.total,
                    "elapsed_ms": r.elapsed_ms,
                    "answer": r.answer,
                    "engines": r.engines,
                })
            })
            .collect();
        println!("{}", json!(out));
    } else {
        output::print_test_report(reports);
    }
    if !any_success {
        std::process::exit(1);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_engines_handles_comma_lists() {
        assert_eq!(split_engines(None), Vec::<String>::new());
        assert_eq!(
            split_engines(Some(" bing, duckduckgo , mojeek ")),
            ["bing", "duckduckgo", "mojeek"]
        );
    }

    #[test]
    fn split_sources_validates_names() {
        assert_eq!(split_sources(None).unwrap(), Vec::<SuggestSource>::new());
        assert_eq!(
            split_sources(Some("duckduckgo, google")).unwrap(),
            [SuggestSource::DuckDuckGo, SuggestSource::Google]
        );
        assert!(split_sources(Some("duckduckgo, nope")).is_err());
        assert!(split_sources(Some("nope")).is_err());
    }

    #[test]
    fn json_score_matches_rest_formula() {
        let resp = phrona::SearchResponse {
            query: "q".into(),
            category: phrona::Category::Web,
            page: 1,
            total: 2,
            results: vec![
                phrona::ResultItem::Web(phrona::WebResult {
                    title: "a".into(),
                    url: "https://a".into(),
                    description: "".into(),
                    engines: vec!["bing".into()],
                    position: 1,
                    score: 1.0,
                }),
                phrona::ResultItem::Image(phrona::ImageResult {
                    title: "i".into(),
                    url: "https://i".into(),
                    image_url: "https://img".into(),
                    thumbnail_url: "".into(),
                    width: 0,
                    height: 0,
                    source: "".into(),
                    engines: vec!["bing_images".into()],
                    position: 2,
                    score: 0.95,
                }),
            ],
            suggestions: vec![],
            answer: None,
            engines: vec![],
            elapsed_ms: 1,
        };
        let v = search_json(&resp);
        assert_eq!(v["results"][0]["position"], 1.0);
        assert_eq!(v["results"][0]["score"], 1.0);
        // image results carry the same score formula as the REST API
        assert_eq!(v["results"][1]["type"], "image");
        assert_eq!(v["results"][1]["score"], 0.95);
        assert_eq!(v["results"][1]["image_url"], "https://img");
        assert_eq!(v["total"], 2);
    }
}
