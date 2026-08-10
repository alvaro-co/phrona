mod args;
mod output;

use anyhow::Result;
use clap::Parser;
use serde_json::json;

use metasearch::SuggestSource;
use metasearch::models::ResultItem;

use args::{Cli, Command, TestArgs};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = metasearch::SearchClient::new()?;

    match &cli.command {
        Command::Search(args) => {
            let mut opts = cli.base_options(&args.query);
            opts.category = args.category;
            opts.engines = split_engines(args.engines.as_deref());
            opts.max_results = args.max_results;
            opts.safesearch = args.safesearch;
            opts.region = args.region.clone();
            opts.language = args.language.clone();
            opts.time_range = args.time_range;
            opts.filters = args.filters.clone();
            opts.page = args.page;
            let resp = client.search(opts).await?;
            if cli.json {
                print_json(&resp);
            } else {
                output::print_response(&resp);
            }
        }
        Command::Suggest(args) => {
            let region = &args.region;
            match split_sources(args.source.as_deref()) {
                Some(sources) => {
                    for s in sources {
                        let list =
                            metasearch::suggest(client.http(), s, &args.query, region).await?;
                        if cli.json {
                            println!("{}", json!({"source": s.name(), "suggestions": list}));
                        } else {
                            println!("{}: {}", s.name(), list.join(" | "));
                        }
                    }
                }
                None => {
                    let all = metasearch::suggest_all(client.http(), &args.query, region).await;
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
                }
            }
        }
        Command::Extract(args) => {
            let page = metasearch::extract(
                client.http(),
                &args.url,
                args.max_chars,
                args.query.as_deref(),
            )
            .await?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&page)?);
            } else {
                println!("title: {}\n", page.title);
                if !page.description.is_empty() {
                    println!("description: {}\n", page.description);
                }
                println!("{}", page.text);
                if !page.images.is_empty() {
                    println!("\nimages: {}", page.images.join(" | "));
                }
            }
        }
        Command::Ground(args) => {
            let mut opts = cli.base_options(&args.query);
            opts.max_results = args.max_results;
            opts.engines = split_engines(args.engines.as_deref());
            let resp = client.search(opts).await?;
            if cli.json {
                print_json(&resp);
            } else {
                output::print_grounded(&args.query, &resp, args.max_results);
            }
        }
        Command::Engines(args) => {
            let cats: Vec<metasearch::Category> = match args.category {
                Some(c) => vec![c],
                None => metasearch::Category::ALL.to_vec(),
            };
            if cli.json {
                let mut map = serde_json::Map::new();
                for c in cats {
                    let names: Vec<String> = metasearch::available_engines(c)
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
            metasearch_mcp::run_stdio().await?;
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
    let api_key = args
        .api_key
        .clone()
        .or_else(|| std::env::var("META_API_KEY").ok().filter(|k| !k.is_empty()));

    let rest_addr = match &args.addr {
        Some(a) => a.parse()?,
        None => std::env::var("META_ADDR")
            .ok()
            .map(|a| a.parse())
            .transpose()?
            .unwrap_or_else(metasearch_api::default_addr),
    };
    let mcp_addr = args.mcp_addr.clone();

    let rest_fut = async {
        if !args.no_rest {
            metasearch_api::serve(rest_addr, api_key.clone()).await?;
        }
        anyhow::Ok(())
    };
    let mcp_fut = async {
        if !args.no_mcp {
            let listener = metasearch_mcp::tcp_listener(&mcp_addr).await?;
            tracing::info!(
                "metasearch-mcp listening on tcp://{mcp_addr} (newline-delimited JSON-RPC)"
            );
            metasearch_mcp::serve_tcp(listener).await?;
        }
        anyhow::Ok(())
    };

    tokio::select! {
        r = rest_fut => r,
        m = mcp_fut => m,
    }
}

fn split_engines(s: Option<&str>) -> Vec<String> {
    s.map(|s| s.split(',').map(|e| e.trim().to_string()).collect())
        .unwrap_or_default()
}

fn split_sources(s: Option<&str>) -> Option<Vec<SuggestSource>> {
    let s = s?;
    let list: Vec<SuggestSource> = s
        .split(',')
        .filter_map(|n| SuggestSource::from_name(n.trim()))
        .collect();
    (!list.is_empty()).then_some(list)
}

fn print_json(resp: &metasearch::SearchResponse) {
    let results: Vec<_> = resp
        .results
        .iter()
        .enumerate()
        .map(|(i, r)| match r {
            ResultItem::Web(w) => json!({
                "type": "web", "title": w.title, "url": w.url,
                "description": w.description, "score": w.score, "position": i + 1,
                "engines": w.engines,
            }),
            ResultItem::Image(im) => json!({
                "type": "image", "title": im.title, "url": im.url,
                "image_url": im.image_url, "thumbnail_url": im.thumbnail_url,
                "width": im.width, "height": im.height, "position": i + 1,
                "engines": im.engines,
            }),
            ResultItem::News(n) => json!({
                "type": "news", "title": n.title, "url": n.url,
                "description": n.description, "published": n.published,
                "source": n.source, "position": i + 1, "engines": n.engines,
            }),
            ResultItem::Video(v) => json!({
                "type": "video", "title": v.title, "url": v.url,
                "description": v.description, "thumbnail_url": v.thumbnail_url,
                "duration": v.duration, "views": v.views, "uploader": v.uploader,
                "position": i + 1, "engines": v.engines,
            }),
            ResultItem::Book(b) => json!({
                "type": "book", "title": b.title, "url": b.url,
                "author": b.author, "publisher": b.publisher, "info": b.info,
                "position": i + 1, "engines": b.engines,
            }),
        })
        .collect();
    println!(
        "{}",
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
    );
}

async fn run_test(cli: &Cli, client: &metasearch::SearchClient, args: &TestArgs) -> Result<()> {
    let cats: Vec<metasearch::Category> = match args.category {
        Some(c) => vec![c],
        None => metasearch::Category::ALL.to_vec(),
    };
    let mut reports = Vec::new();
    for cat in cats {
        let mut opts = cli.base_options(&args.query);
        opts.category = cat;
        opts.max_results = 5;
        match client.search(opts).await {
            Ok(resp) => reports.push((cat, resp)),
            Err(e) => {
                reports.push((
                    cat,
                    metasearch::SearchResponse {
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
                    "engines": r.engines,
                })
            })
            .collect();
        println!("{}", json!(out));
    } else {
        output::print_test_report(reports);
    }
    Ok(())
}
