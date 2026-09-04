# Architecture

Six crates, one core. Every surface is a thin composition over the
`phrona` library; nothing in the core knows about HTTP servers, MCP,
Python or the web UI.

```text
                  phrona (core library)
                 engines | dedup | rank | extract | suggest
                   client | options | models | parse
        |              |              |              |
  phrona-api   phrona-mcp  phrona-python  phrona-cli
   (axum REST)      (rmcp MCP)      (pyo3 bindings)   (phrona: all-in-one)
        |              |                |                |
      assets/      stdio server      wheel (uv)     + embeds api+mcp
      (static SPA)  + tcp server
```

## Layering rules

- `phrona` depends on nothing from the workspace (wreq, serde,
  scraper, tokio, ... only).
- `phrona-api` depends on `phrona`; exposes `router(PhronaConfig)` and
  `serve(addr, cfg)`. Its binary is a thin
  wrapper (`cargo run -p phrona-api`).
- `phrona-mcp` depends on `phrona`; exposes `run_stdio(&cfg)` and
  `serve_tcp(listener, cfg, shutdown)`. Its binary is a thin wrapper.
- `phrona-cli` depends on all three above and composes them: search
  etc. use the core directly, `phrona serve` runs the REST router and the
  MCP TCP listener in one tokio runtime, `phrona mcp` runs the stdio server.
- `phrona-python` depends on `phrona` (aliased `phrona`
  to avoid cdylib name collision) and is packaged as a wheel.
- `examples/rust` depends only on `phrona`.

Composition instead of duplication: there is exactly one implementation of
search, merging, ranking, extraction, the REST routes and the MCP tools;
every interface is a different door into the same code.

## Key components (core)

| Module | Responsibility |
| --- | --- |
| `client` | wreq wrapper: impersonation profiles, cookie jar, redirects, optional proxy, timeouts |
| `engines` | 29 engine modules + solvers (`altcha`, `anubis`), `suggest` (7 sources) and shared `util`, each stateless and testable via fixtures |
| `engine` | `Engine` trait, per-search context, shared token caches (DDG vqd, Startpage sc) |
| `dedup` | URL normalization, tracking-param stripping, cross-engine grouping |
| `rank` | agreement + position + text-match scoring, wikipedia bonus |
| `search` | streaming fan-out (`FuturesUnordered`), adaptive deadline + early exit, per-engine error isolation, answer routing, sync/async |
| `extract` | readable-text extraction and query-biased excerpts (grounding) |
| `options` | `SearchOptions` with categories, regions, time ranges, safesearch, filters |
| `models` | `ResultItem` union (web/image/news/video/book), `SearchResponse`, `EngineReport` |
| `error` | structured `Error { scope, kind, engine, http_status, message }` — allocation-free, typed `From<wreq::Error>` |

## Availability design

- Engines run concurrently (`FuturesUnordered`) under one adaptive deadline
  (`SearchOptions.timeout`); as soon as the merged set reaches
  `max_results`, remaining in-flight futures are cancelled and the search
  returns early. One failing engine never blocks the others; failures are
  reported per engine in the response (`EngineReport.status/error/scope/kind`).
- Every response is classified from HTTP semantics alone (status code,
  anti-bot headers like `cf-mitigated`/`cf-ray`, `Retry-After`, and
  `Content-Type`) in `util::check_response`, never from body phrasing; a 2xx
  page that the parser can't turn into results degrades to zero gracefully.
- Errors carry a `scope` (egress block vs provider outage vs schema drift vs
  query problem vs internal) plus the observable `kind`, so callers can
  react differently; an error is only raised when *every* engine failed
  (`AllProvidersFailed`), otherwise empty results are honest.
- The `test` command and `/health` expose live availability for
  monitoring.
- The `upstream-watch` workflow detects when the scraped upstream
  projects change, so broken parsers are caught early.

## Adding a surface

A new interface (e.g. a TUI or a plugin system) depends on `phrona`
and reuses `SearchClient`/`search`/`extract`/`suggest`. No core changes
are needed unless a new engine or feature is added, in which case the
pattern is: engine module + fixture + `parse_fixture` test (see
[docs/engines.md](engines.md)).
