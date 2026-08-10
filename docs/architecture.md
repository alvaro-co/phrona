# Architecture

Six crates, one core. Every surface is a thin composition over the
`metasearch` library; nothing in the core knows about HTTP servers, MCP,
Python or the web UI.

```text
                  metasearch (core library)
                 engines | dedup | rank | extract | suggest
                   client | options | models | parse
        |              |              |              |
  metasearch-api   metasearch-mcp  metasearch-python  metasearch-cli
   (axum REST)      (rmcp MCP)      (pyo3 bindings)   (ms: all-in-one)
        |              |                |                |
      frontend/    stdio server      wheel (uv)     + embedds api+mcp
      (static SPA)  + tcp server
```

## Layering rules

- `metasearch` depends on nothing from the workspace (wreq, serde,
  scraper, tokio, ... only).
- `metasearch-api` depends on `metasearch`; exposes `router(api_key)`,
  `serve(addr, api_key)` and `serve_from_env()`. Its binary is a thin
  wrapper (`cargo run -p metasearch-api`).
- `metasearch-mcp` depends on `metasearch`; exposes `run_stdio()` and
  `serve_tcp(listener)`. Its binary is a thin wrapper.
- `metasearch-cli` depends on all three above and composes them: search
  etc. use the core directly, `ms serve` runs the REST router and the
  MCP TCP listener in one tokio runtime, `ms mcp` runs the stdio server.
- `metasearch-python` depends on `metasearch` (aliased `metasearch-core`
  to avoid cdylib name collision) and is packaged as a wheel.
- `examples/rust` depends only on `metasearch`.

Composition instead of duplication: there is exactly one implementation of
search, merging, ranking, extraction, the REST routes and the MCP tools;
every interface is a different door into the same code.

## Key components (core)

| Module | Responsibility |
| --- | --- |
| `client` | wreq wrapper: impersonation profiles, cookies, redirects, proxies, timeouts |
| `engines` | 25 engine modules + 7 suggestion sources, each stateless and testable via fixtures |
| `engine` | `Engine` trait, per-search context, shared token caches (DDG vqd, Startpage sc) |
| `dedup` | URL normalization, tracking-param stripping, cross-engine grouping |
| `rank` | agreement + position + text-match scoring, wikipedia bonus |
| `search` | parallel fan-out, per-engine error isolation, answer routing, sync/async |
| `extract` | readable-text extraction and query-biased excerpts (grounding) |
| `options` | `SearchOptions` with categories, regions, time ranges, safesearch, filters, profiles, proxies |
| `models` | `ResultItem` union (web/image/news/video/book), `SearchResponse`, `EngineReport` |

## Availability design

- Engines run in parallel and are isolated: one failing engine never
  blocks the others; failures are reported per engine in the response.
- `is_block_page` keeps captcha/rate-limit pages out of fixtures and
  lets parsers degrade to zero results gracefully.
- The `test` command and `/health` expose live availability for
  monitoring.
- The `upstream-watch` workflow detects when the scraped upstream
  projects change, so broken parsers are caught early.

## Adding a surface

A new interface (e.g. a TUI or a plugin system) depends on `metasearch`
and reuses `SearchClient`/`search`/`extract`/`suggest`. No core changes
are needed unless a new engine or feature is added, in which case the
pattern is: engine module + fixture + `parse_fixture` test (see
[docs/engines.md](engines.md)).
