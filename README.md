# Phrona

High-performance metasearch engine library written in Rust. Queries 26 search
engines across 5 categories in parallel, impersonates real browsers over
HTTP/2 (wreq), merges, deduplicates and ranks results locally, and exposes
the same engine to Rust, Python, a REST API, an MCP server for AI agents and
a web frontend.

```text
crates/phrona          core library
crates/phrona-api      REST API server (axum)
crates/phrona-mcp      MCP server for AI agents (rmcp, stdio + TCP)
crates/phrona-cli      phrona: single CLI to everything (search, server, MCP)
crates/phrona-python   Python bindings (pyo3)
examples/                  runnable Rust and Python examples
crates/phrona-api/assets/      Material 3 style static web app
scripts/                   upstream drift monitor
docs/                      full documentation
```

## Quick start (CLI)

```bash
cargo run -p phrona-cli -- search "rust programming" --max-results 10
cargo run -p phrona-cli -- suggest rus
cargo run -p phrona-cli -- serve         # REST 8080 + MCP-over-TCP 8081
```

`phrona` is the all-in-one entry point: search, suggest, extract, grounding,
engine listing, availability tests (`phrona test`), the full REST server, MCP
over stdio (`phrona mcp`) or TCP (`phrona serve`), and shell completions. See
[docs/cli.md](docs/cli.md).

## Quick start (Rust)

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

## Quick start (REST API)

```bash
cargo run -p phrona-api                 # listens on 127.0.0.1:8080
curl "localhost:8080/v1/search?q=rust&max_results=5"
```

## Quick start (Python)

```bash
uv build                                    # produces dist/phrona-*.whl
uv pip install dist/phrona-*.whl --python <venv python3.12>
```

```python
import phrona
phrona.search("rust programming", engines=["bing", "brave"])
phrona.suggest("rus")
```

Requires CPython <= 3.13 (pyo3 0.29).

Pre-built wheels are published to PyPI for CPython 3.9-3.13 on linux
(x86_64 / aarch64), macOS (x86_64 / arm64) and Windows (x86_64), plus an
sdist for everything else (needs a Rust toolchain + `uv build` / maturin).
No musllinux wheels (btls-sys requires a musl C++ toolchain).

## Quick start (MCP)

```bash
cargo run -p phrona-mcp                 # stdio JSON-RPC
cargo run -p phrona-cli -- mcp          # same, via phrona
cargo run -p phrona-cli -- serve        # also serves MCP over TCP 8081
```

Point your MCP client (Claude Desktop, claude-code, Cursor...) at the
`phrona-mcp` binary. Nine tools: `web_search`, `image_search`,
`news_search`, `video_search`, `book_search`, `suggest`, `fetch_page`,
`search_grounded`, `list_engines`.

## Quick start (web)

```bash
cargo run -p phrona-api          # or: phrona serve
# open http://localhost:8080         -> search page (full parameters + JSON view)
#                                    -> Tools tab: suggest / extract / ground / engines / test
```

The web app is served from disk (`crates/phrona-api/assets/`), so editing it takes effect
on reload - no rebuild.

## Features

- 26 engines, 5 categories (web, images, news, videos, books), 1 suggestion
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
  page extraction, availability probing (`/v1/test`), suggestions and health.
- MCP server with compartmentalized tools and grounded search for RAG.
- Python bindings with the full search surface.
- Web frontend: one static page with full-parameter search (engines,
  category, safesearch, region, language, time range, filters, page,
  JSON view, per-engine report) and a Tools tab that runs every CLI
  capability in the browser (suggest, extract, ground, engines, test).
- Fixture-based parser tests: 26 captured live pages (see
  `crates/phrona/tests/fixtures/`), network-independent.
- Upstream drift monitor: `scripts/watch_upstream.sh` + the
  `upstream-watch` GitHub workflow report when any of the 8 upstream
  projects moves past its pinned commit, so broken parsers are caught
  early (see [docs/upstream.md](docs/upstream.md)).
- Tag-driven releases: push `vX.Y.Z` and the `release` workflow builds
  binaries for Linux/Windows/macOS plus the Python wheel and publishes a
  GitHub Release (see [docs/releasing.md](docs/releasing.md)).

## Layout of this documentation

| File | Contents |
| --- | --- |
| [docs/library.md](docs/library.md) | Rust library reference |
| [docs/api.md](docs/api.md) | REST API reference |
| [docs/mcp.md](docs/mcp.md) | MCP server reference |
| [docs/python.md](docs/python.md) | Python bindings reference |
| [docs/cli.md](docs/cli.md) | CLI (`phrona`) reference |
| [docs/architecture.md](docs/architecture.md) | Architecture and layering |
| [docs/examples.md](docs/examples.md) | Rust/Python examples |
| [docs/frontend.md](docs/frontend.md) | Web frontend |
| [docs/engines.md](docs/engines.md) | Engine-by-engine reference and block status |
| [docs/upstream.md](docs/upstream.md) | Upstream sources, what was borrowed, how to monitor |
| [docs/releasing.md](docs/releasing.md) | Tagging and publishing releases |
| [docs/HISTORY.md](docs/HISTORY.md) | Complete build history |

## Development

Both `make` and plain `cargo` work; the cargo commands are the same ones
CI runs:

```bash
cargo fmt --all --check           # or: make fmt-check
cargo clippy --workspace --all-targets -- -D warnings   # or: make lint
cargo test --workspace            # 77 offline tests incl. 26 fixtures, ~1 s  (make test)
make check                        # fmt-check + lint + test in one

cargo build --workspace           # or: make build
cargo build --release -p phrona-cli -p phrona-api -p phrona-mcp  # make release
uv build                          # Python wheel (make wheel)

cargo run -p phrona --bin fetch_fixtures [engine...]   # re-capture live fixtures
cargo run -p phrona --bin dbg_parse -- bing            # parse a fixture
cargo run -p phrona-examples --bin basic -- "rust"     # run the examples
```

Known limitation: Google, Qwant, Mojeek and the DDG HTML endpoint
anti-bot-block this network (429 / CAPTCHA / 403), so their parsers are
tested only for graceful behavior and their engines may need proxies or a
cleaner IP. See [docs/engines.md](docs/engines.md).

## License

[AGPL-3.0](LICENSE)
