mod args;
mod output;

use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use serde_json::json;

use phrona::SuggestSource;
use phrona::config::PhronaConfig;

use args::{Cli, Command, TestArgs};

/// Load the typed configuration; a broken file degrades to defaults with a
/// warning so the CLI stays usable.
fn load_config() -> PhronaConfig {
    match PhronaConfig::load() {
        Ok(cfg) => cfg,
        Err(e) => {
            eprintln!("config warning: {e}");
            PhronaConfig::defaults()
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let cfg = load_config();
    let profile = cli.profile.unwrap_or_else(|| cfg.profile());
    let timeout = Duration::from_secs(cli.timeout.unwrap_or(cfg.search.timeout_secs));
    let proxies = if cli.proxy.is_empty() {
        cfg.engines.proxies.clone()
    } else {
        cli.proxy.clone()
    };
    let client = phrona::SearchClient::with_options(
        profile,
        Some(timeout),
        (!proxies.is_empty()).then_some(proxies),
        phrona::TargetPolicy::default(),
    )?;

    match &cli.command {
        Command::Search(args) => {
            let mut opts = cli.base_options(timeout, &args.query);
            opts.category = args.category;
            opts.engines = split_engines(args.engines.as_deref());
            opts.max_results = args.max_results.clamp(1, cfg.max_results_limit());
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
            let mut opts = cli.base_options(timeout, &args.query);
            opts.max_results = args.max_results.clamp(1, cfg.max_results_limit());
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
            run_test(&cli, &client, args, timeout).await?;
        }
        Command::Serve(args) => {
            run_serve(args, &cfg).await?;
        }
        Command::Mcp => {
            init_tracing();
            phrona_mcp::run_stdio(&cfg).await?;
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

/// Full server: REST API (axum) plus MCP-over-TCP, in one process. Addresses
/// and the API key default to `server.bind_addr` / `server.mcp_addr` /
/// `server.api_key` from the config (env overrides included).
async fn run_serve(args: &args::ServeArgs, cfg: &PhronaConfig) -> anyhow::Result<()> {
    init_tracing();
    let mut cfg = cfg.clone();
    if let Some(k) = &args.api_key {
        cfg.server.api_key = Some(k.clone());
    }

    let rest_addr = match &args.addr {
        Some(a) => a.parse()?,
        None => std::env::var("PHRONA_ADDR")
            .ok()
            .filter(|a| !a.is_empty())
            .map(|a| a.parse())
            .transpose()?
            .unwrap_or(cfg.bind_addr()?),
    };
    let mcp_addr = args
        .mcp_addr
        .clone()
        .unwrap_or_else(|| cfg.server.mcp_addr.clone());

    // One shared shutdown trigger: SIGTERM/Ctrl+C fans out to the REST
    // server (graceful drain), the MCP TCP server (accept-loop stop + drain
    // window) and the select arms below.
    let shutdown = std::sync::Arc::new(tokio::sync::Notify::new());
    {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            phrona_api::shutdown_signal().await;
            shutdown.notify_waiters();
        });
    }

    let rest_fut = async {
        if !args.no_rest {
            phrona_api::serve(rest_addr, cfg.clone()).await?;
        }
        anyhow::Ok(())
    };
    let mcp_fut = async {
        if !args.no_mcp {
            let listener = phrona_mcp::tcp_listener(&mcp_addr).await?;
            tracing::info!("phrona-mcp listening on tcp://{mcp_addr} (newline-delimited JSON-RPC)");
            phrona_mcp::serve_tcp(listener, cfg.clone(), shutdown.clone()).await?;
        }
        anyhow::Ok(())
    };

    match (!args.no_rest, !args.no_mcp) {
        // Both listeners: SIGTERM/Ctrl+C drains the REST server gracefully
        // (axum waits for in-flight requests) and then the process exits.
        // `biased` prefers the completed REST future so the drain is never
        // preempted by the signal branch.
        (true, true) => {
            tokio::select! {
                biased;
                r = rest_fut => r?,
                m = mcp_fut => m?,
                _ = shutdown.notified() => {}
            }
        }
        (true, false) => {
            tokio::select! {
                biased;
                r = rest_fut => r?,
                _ = shutdown.notified() => {}
            }
        }
        (false, true) => {
            tokio::select! {
                biased;
                m = mcp_fut => m?,
                _ = shutdown.notified() => {}
            }
        }
        (false, false) => {
            anyhow::bail!("nothing to serve: both --no-rest and --no-mcp are set");
        }
    }
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
    println!(
        "{}",
        serde_json::to_string_pretty(&search_json(resp)).expect("serialize response")
    );
}

/// Machine-readable JSON for a search response: the canonical serde
/// serialization of [`phrona::SearchResponse`]. Pure (no stdout), so the
/// exact payload is unit-testable.
fn search_json(resp: &phrona::SearchResponse) -> serde_json::Value {
    serde_json::to_value(resp).expect("search response is serializable")
}

async fn run_test(
    cli: &Cli,
    client: &phrona::SearchClient,
    args: &TestArgs,
    timeout: Duration,
) -> Result<()> {
    let cats: Vec<phrona::Category> = match args.category {
        Some(c) => vec![c],
        None => phrona::Category::ALL.to_vec(),
    };
    let mut reports = Vec::new();
    let mut any_success = false;
    for cat in cats {
        let mut opts = cli.base_options(timeout, &args.query);
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
    fn json_is_canonical_serde_serialization() {
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
        // identical to the derive-based canonical serialization
        assert_eq!(v, serde_json::to_value(&resp).unwrap());
        assert_eq!(v["results"][0]["position"], 1);
        assert_eq!(v["results"][0]["score"], 1.0);
        // every variant serializes through the same tagged enum
        assert_eq!(v["results"][1]["type"], "image");
        assert_eq!(v["results"][1]["score"], 0.95);
        assert_eq!(v["results"][1]["image_url"], "https://img");
        assert_eq!(v["total"], 2);
    }
}
