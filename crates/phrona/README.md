# phrona

High-performance metasearch engine library for Rust. Queries 29 search
engines across 8 categories (web, images, news, videos, books, code,
papers, archives) in parallel,
impersonates real browsers over HTTP/2, then merges, deduplicates and ranks
results locally.

## Quick start

```rust
use phrona::{search_sync, SearchOptions};

let resp = search_sync(SearchOptions::new("rust programming"))?;
for r in resp.web() {
    println!("{} {}", r.title, r.url);
}
```

Async:

```rust
use phrona::{SearchClient, SearchOptions};

let client = SearchClient::new()?;
let resp = client.search(SearchOptions::new("rust programming")).await?;
```

## Features

- 29 engines, 8 categories, suggestions and page extraction (AI grounding).
- Impersonated HTTP/2 with TLS fingerprint spoofing via wreq profiles
  (Chrome, Firefox, Safari, Edge, Opera, OkHttp).
- Cross-engine merging: tracking-parameter stripping, dedup, ranking,
  per-engine error reporting in every response.
- Sync and async APIs, time ranges, safesearch, regions, per-engine filter
  strings, proxies.

## Related crates

- [phrona-api](https://crates.io/crates/phrona-api) — REST API server (axum)
- [phrona-mcp](https://crates.io/crates/phrona-mcp) — MCP server for AI agents
- [phrona-cli](https://crates.io/crates/phrona-cli) — the `phrona` binary
- [phrona-python](https://pypi.org/project/phrona/) — Python bindings

## Documentation

Full docs (library reference, engines, examples) live in the main repository:
<https://github.com/alvaro-co/phrona/tree/main/docs>

## License

AGPL-3.0 — see the main repository for details.