# MetaSearchRS

High-performance metasearch engine library written in Rust. Queries 25 search
engines across 5 categories in parallel, impersonates real browsers over
HTTP/2 (wreq), merges, deduplicates and ranks results locally, and exposes
the same engine to Rust, Python, a REST API, an MCP server for AI agents and
a web frontend.

```text
crates/metasearch          core library (Rust)
crates/metasearch-api      REST API server (axum)
crates/metasearch-mcp      MCP server for AI agents (rmcp, stdio)
crates/metasearch-python   Python bindings (pyo3)
frontend/                  Material 3 style static web app
docs/                      full documentation
```

## Quick start (Rust)

```rust
use metasearch::{search_sync, SearchOptions};

let resp = search_sync(SearchOptions::new("rust programming"))?;
for r in resp.web() {
    println!("{} {}", r.title, r.url);
}
```

Async:

```rust
use metasearch::{SearchClient, SearchOptions};

let client = SearchClient::new()?;
let resp = client.search(SearchOptions::new("rust programming")).await?;
```

## Quick start (REST API)

```bash
cargo run -p metasearch-api                 # listens on 127.0.0.1:8080
curl "localhost:8080/v1/search?q=rust&max_results=5"
```

## Quick start (Python)

```bash
uv build                                    # produces dist/metasearch-*.whl
uv pip install dist/metasearch-*.whl --python <venv python3.12>
```

```python
import metasearch
metasearch.search("rust programming", engines=["bing", "brave"])
metasearch.suggest("rus")
```

Requires CPython <= 3.13 (pyo3 0.24 ABI).

## Quick start (MCP)

```bash
cargo run -p metasearch-mcp                 # stdio JSON-RPC
```

Point your MCP client (Claude Desktop, claude-code, Cursor...) at the
`metasearch-mcp` binary. Nine tools: `web_search`, `image_search`,
`news_search`, `video_search`, `book_search`, `suggest`, `fetch_page`,
`search_grounded`, `list_engines`.

## Quick start (web)

Start the API server and open http://localhost:8080.

## Features

- 25 engines, 5 categories (web, images, news, videos, books), 1 suggestion
  source family and page extraction (AI grounding).
- Impersonated HTTP/2 with TLS fingerprint spoofing via wreq profiles
  (Chrome 100-149, Firefox 139-148, Safari 26, Edge 148, Opera 131, OkHttp).
- Search merging: cross-engine dedup (tracking parameters stripped), ranking
  (cross-engine agreement + position + query text match), suggestions and
  per-engine error reporting in every response.
- Full sync and async APIs, `SearchClient` reuse with shared token caches
  (DDG vqd, Startpage sc), time ranges, safesearch, regions, per-engine
  filter strings, proxies.
- REST API with Tavily-compatible `/search` endpoint, AI grounding endpoint,
  suggestions and health.
- MCP server with compartmentalized tools and grounded search for RAG.
- Python bindings with the full search surface.
- Fixture-based parser tests: 25 captured live pages (see
  `crates/metasearch/tests/fixtures/`), network-independent.

## Layout of this documentation

| File | Contents |
| --- | --- |
| [docs/library.md](docs/library.md) | Rust library reference |
| [docs/api.md](docs/api.md) | REST API reference |
| [docs/mcp.md](docs/mcp.md) | MCP server reference |
| [docs/python.md](docs/python.md) | Python bindings reference |
| [docs/frontend.md](docs/frontend.md) | Web frontend |
| [docs/engines.md](docs/engines.md) | Engine-by-engine reference and block status |
| [docs/upstream.md](docs/upstream.md) | Upstream sources, what was borrowed, how to monitor |
| [docs/HISTORY.md](docs/HISTORY.md) | Complete build history |

## Development

```bash
cargo test --workspace     # 29 parser/merge unit tests (offline, fixtures)
cargo run -p metasearch --bin fetch_fixtures   # re-capture live fixtures
cargo run -p metasearch --bin dbg_parse -- bing # parse a fixture, show results
cargo clippy --all-targets
cargo fmt
```

Known limitation: Google, Qwant, Mojeek and the DDG HTML endpoint
anti-bot-block this network (429 / CAPTCHA / 403), so their parsers are
tested only for graceful behavior and their engines may need proxies or a
cleaner IP. See [docs/engines.md](docs/engines.md).

## License

MIT
