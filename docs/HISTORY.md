# HISTORY

A log of how MetaSearchRS was built, commit by commit.

## b50d936 - chore: scaffold workspace with wreq-based impersonating HTTP client

- Workspace `MetaSearchRS` with a single library crate `metasearch`.
- `HttpClient`/`HttpClientBuilder` wrapping `wreq 6.0.0-rc.29` +
  `wreq-util 3.0.0-rc.14`: TLS fingerprint spoofing via `Profile`
  (Chrome 100-149, Firefox 139-148, Safari 26, Edge 148, Opera 131,
  OkHttp, Random), cookie jar, redirects, custom headers, timeouts,
  proxy pool support.
- Decision: pick wreq over reqwest/hyper-tls because TLS fingerprinting
  and HTTP/2 impersonation are the only realistic defense against bot
  detection when scraping the big engines.
- Decision: fetch **live fixtures** from the real engines (no mock HTML
  invented by hand), because hand-written fixtures hide parser bugs.

## (engine development phase)

Engine-by-engine implementation, each engine in
`crates/metasearch/src/engines/*.rs`, driven by the fixture loop:
`fetch_fixtures <engine>` -> `dbg_parse <engine>` -> fix parser ->
commit. 25 engines across web/images/news/videos/books plus 7
suggestion sources (DuckDuckGo, Google, Bing, Brave, Startpage, Qwant,
Wikipedia).

Highlights and hard-won details:

- **bing**: HTML results; `first=` paging; news via the
  `infinitescrollajax` JSON endpoint (parallel headline fetch for the
  current top article); videos via `asyncv2`.
- **bing_videos**: all result markup lives inside `<noscript>` - html5ever
  treats it as raw text, so a regex unwraps the tags before DOM parsing;
  the thumbnail signature is read directly off the node as a fallback.
- **brave**: results embedded as JSON blobs in `data-serpapi` attributes;
  `js_to_json` converts JS object literals to strict JSON; thumbnails use
  base64-encoded dims (`/g:ce/`) that need URL-safe *and* standard base64
  decoding; page 2 needs `spellcheck=false&s=` continuation.
- **duckduckgo**: the **vqd token dance** (token from
  `duckduckgo.com/?vqd=`, then sign `/html/` and the `i.js`/`news.js`/
  `v.js` JSON endpoints); token cached in `EngineShared` (1 h TTL).
- **startpage**: the **sc token dance** (GET the search page to receive
  the token, then POST the search; token cached); images via POST
  `tbm=isch`; suggestions from `/suggestions`.
- **qwant**: JSON API `api.qwant.com/v3/search/web` + `/suggest`.
- **yandex**: only the site-restricted `search/site/` variant parses
  reliably (the main search returns an anti-bot page).
- **wikipedia**: OpenSearch API + cross-article URL expansion.
- **grokipedia**: typeahead + `/page/` fetches, populates the `answer`
  field (one-shot answer engine, SearXNG "answerer" pattern).
- **google**: `udm=14` for plain HTML, `tbm=isch` for images.
- **annas_archive**: HTML book listing with author/publisher/info.
- **mojeek**, **yahoo/yahoo_news**: HTML; images served through the same
  HTML page as web results.

Hard truth learned: this network's IP is rate-limited/captcha'd by
Google (429 enablejs), Mojeek (403), Qwant (DataDome) and DuckDuckGo
(anomaly page). These are *environmental*, not parser bugs. Introduced
`is_block_page()` to recognize bot pages, so fixtures never get polluted
and parser tests skip block pages gracefully.

## c8eb2a2 - engines: fix bing_videos noscript parsing, tolerant tests for bot-blocked fixtures, clippy cleanup

- `regex_strip_noscript` fix for bing videos (see above).
- All `parse_fixture` tests guard on `is_block_page`; `fetch_fixtures`
  never overwrites good fixtures with block pages.
- `brave_b64_decode` handles standard base64.
- Clippy cleanup across engines.

## c42caff - fetch_fixtures: fix Job future lifetime for non-static engine requests

- The fixture-fetching dev tool generalized the search invocation to
  per-engine requests; the `Job` future needed an explicit lifetime
  bound (`Pin<Box<dyn Future + Send + 'a>>`).

## (merging, dedup, ranking, options, client consolidation)

- `dedup`: `dedup_key` normalizes URLs (lowercase host, tracking-param
  stripping: utm_*, fbclid, gclid, ref, source, si, spm, s, ...);
  `group` separates empty-URL items (answer markers) from real results.
- `rank`: score = cross-engine agreement (+1.5 per extra engine, capped)
  + position (10/pos) + Wikipedia/Grokipedia bonus + query-term text
  match.
- Critical fix: `search.rs` used `items.drain(..).filter(...)` which
  *emptied* the raw-results vector and dropped every result - replaced
  with `into_iter().partition(...)`; regression test
  `merge_keeps_results_and_answers` added. Live proof: real Bing results
  flow end-to-end through library, API and MCP.
- `SearchOptions` gained `Category`/`SafeSearch`/`TimeRange` as
  `FromStr` types; full sync + async APIs; `SearchClient` for connection
  reuse.

## 0aafbc9 - api: axum REST server with search, suggest, engines, health, Tavily-compatible /search, AI grounding

- `metasearch-api` (axum 0.8 + tower-http):
  - `GET /` (frontend), `/health`, `/v1/engines`, `/v1/search`,
    `/v1/suggest`, `GET|POST /v1/grounding`.
  - `POST /search` + `POST /v1/tavily`: Tavily-compatible endpoint
    (search_depth, topic, days, include_images/answer/raw_content,
    include/exclude_domains) so Tavily clients work unchanged.
  - Optional auth via `META_API_KEY` (header/query/body); permissive
    CORS; tracing.
  - Decision: serve the frontend as **static files from the repo dir**
    (no embedding, no build step) - `frontend.rs` maps known asset
    paths, everything else falls back to the app shell.

## 5fb62eb - mcp: Model Context Protocol stdio server exposing metasearch tools to AI agents

- `metasearch-mcp` (rmcp 3.1, features `server` + `transport-io`,
  schemars, anyhow): 9 tools - web_search, image_search, news_search,
  video_search, book_search, suggest, fetch_page, search_grounded,
  list_engines.
- Verified by hand over JSON-RPC stdio: `initialize` (protocol
  2025-11-25), `tools/list`, `tools/call` returning real live results.
- `search_grounded` implements the RAG pattern: search, score, fetch the
  best page, return a verbatim excerpt + ranked sources.

## 83901ce - python: pyo3 bindings with setuptools-rust wheel packaging (uv build verified)

- `metasearch-python`: pyo3 0.24; cdylib named `metasearch`; the
  internal Rust crate is aliased `metasearch-core` to avoid a name
  collision on the produced artifacts.
- `Client` pyclass (profile + timeout, full SearchOptions kwargs) plus
  module-level `search`/`suggest`/`extract`/`engines`/`version`;
  `json_to_py` returns native Python dicts/lists.
- `ExtractedPage` got a manual `Serialize` impl so it can flow through
  the same JSON conversion path.
- Wheel packaging: workspace `pyproject.toml` with setuptools-rust,
  `[[tool.setuptools-rust.ext-modules]]` keyed by target name; verified
  with `uv build` -> `dist/metasearch-0.1.0-cp312-cp312-linux_x86_64.whl`
  -> fresh venv install -> all functions called successfully.
- Verified quirk: pyo3 0.24 segfaults on CPython 3.14; Python <= 3.13
  only.

## 9d05a85 - frontend: Material 3 style static SPA served by the API server

- `frontend/`: `index.html` + `style.css` (Material 3 tokens, light/dark
  themes, chips, cards, responsive image grid) + `app.js` (debounced
  suggestions, category/engine chips, region/language/time-range/max
  results controls, answer banner, error banner, theme persistence).
- API fallback fixed to serve assets by request URI path (the axum
  `Path<Vec<String>>` extractor receives an empty vec in fallback
  handlers - read `req.uri().path()` instead).

## 094d8e9 - answer: grokipedia emits an answer marker; Tavily include_answer queries the answer engine

- The `answer` field was dead: no engine ever returned a URL-less marker,
  so `SearchResponse.answer` was always None and Tavily's `include_answer`
  never produced content despite being documented.
- grokipedia now returns two raw results: an answer marker (empty URL,
  typeahead snippet capped at a sentence boundary, 500 chars) and the
  page result itself. The merge partition routes the marker to the
  answer field verbatim.
- Tavily `/search` with `search_depth=basic` appends the grokipedia answer
  engine to the engine set when `include_answer` is requested (basic depth
  previously restricted engines to bing+duckduckgo, so the answer engine
  never ran).
- docs: grounding and search_grounded response shapes corrected to the
  implemented {query, answer, sources[]} contract.

## 00990e1 - tests: 35 new offline tests (64 total); five real bugs fixed

New test modules per subsystem (dedup, rank, options, util, suggest,
extract), all fixture-free and fast (0.5 s). Bugs surfaced and fixed:

- google suggest with `<b>` emphasis lost the prefix (only the bold
  fragment was returned); the full suggestion text is taken now.
- js_to_json left single-quoted strings unterminated (closing quote was
  not converted to `"`) and quoted numeric values; both fixed.
- meta description extraction read element text (always empty) instead
  of the content attribute.
- truncate emitted max chars plus `...` (overran the budget); the
  ellipsis now counts toward max.
- SafeSearch FromStr silently mapped unknown values to Moderate; now
  strict, and the REST API returns 400 for invalid safesearch.
- wikipedia/grokipedia ranking bonus raised 2.0 -> 3.0 so a wiki hit at
  position 10 deterministically outranks a same-text position-1 hit.

## Final pass

- `cargo fmt`, `cargo clippy --all-targets` (library, API, MCP: zero
  warnings), `cargo test --workspace` (64 tests, offline via fixtures,
  ~0.5 s), `cargo build --release`.
- End-to-end smoke: REST API on a scratch port (health, search with live
  bing/brave/yahoo/wikipedia/grokipedia results, suggest from all 7
  sources, grounding with real answers, Tavily with include_answer,
  parameter validation 400s, frontend assets), MCP JSON-RPC session
  (initialize, tools/list, tools/call including search_grounded with
  live answers), Python crate `cargo check`.

## Final pass: CLI crate, examples, upstream watch

- **metasearch-cli (`ms`)** - one binary for every surface: `search`,
  `suggest`, `extract`, `ground`, `engines`, `test` (live availability
  probe), `serve` (REST + MCP-over-TCP in one process), `mcp` (stdio),
  `completions`. Generated completions via clap_complete; `--json`
  everywhere; the merged response carries the per-engine report.
- **`crates/metasearch-api/src/lib.rs`** and
  **`crates/metasearch-mcp/src/lib.rs`** extracted so both servers are
  embeddable libraries (axum `router()/serve()`, rmcp
  `run_stdio()/serve_tcp()`) - the CLI composes them in one tokio
  runtime instead of duplicating.
- **examples/rust** workspace crate (basic, suggest, extract, ground)
  and **examples/python** scripts (module-level functions and a
  reusable `Client`), both exercising only the public library API.
- **Makefile** - `make check/build/release/wheel/examples/serve`.
- **Upstream drift monitor** (the spec's "watch for changes on the
  upstream repos" requirement): `scripts/upstream-refs.txt` pins the 8
  upstream projects to the commits they were verified against;
  `scripts/watch_upstream.sh` clones each pin and checks whether it is
  still an ancestor of the upstream tip; the `upstream-watch` GitHub
  workflow runs it weekly and opens/updates a GitHub issue listing the
  drifted repos (also reachable via `workflow_dispatch`). The watch
  script depends only on git + python3 (no jq).
- **Pin correction**: the first run of the watch caught that the ddgs
  and websurfx pins were swapped (each repo's tip matched the other's
  pin exactly), and that 4get, searxng, primp, wreq, wreq-util and
  mcp-4get had moved. Pins updated to verified tips; a note in
  docs/upstream.md records the correction. Verification shows zero
  drift at the new pins.

## Final pass: full parameter surface, web tools tab, release workflow

- **API completeness**: `/v1/engines` accepts an optional `category`;
  new `/v1/extract` (GET/POST, the library's `extract` feature) and
  `/v1/test` (the `ms test` availability probe) so every CLI capability
  is reachable over HTTP.
- **CLI completeness**: `ms extract` takes multiple URLs (parallel via
  `extract_many`); `ms ground` accepts the full option set (category,
  region, language, time range, safesearch, filters, page); `ms test`
  gained `--max-results`.
- **MCP completeness**: the search tools now expose `safesearch`,
  `language`, `filters` and `page` (the docs already promised them).
- **Frontend redesign** (one static page, zero bloat): Search tab with
  every library parameter - engines, category, safesearch, region,
  language, time range, filters, page, suggestions toggle, JSON view,
  collapsible per-engine report, pagination, and the full query kept in
  the URL hash (shareable results). Tools tab runs the five CLI
  operations in the browser (suggest/extract/ground/engines/test), each
  against the live API with a JSON toggle. Vanilla JS, one file, no
  framework, no build step. WASM evaluation: rejected - wreq/hyper's
  TLS layer cannot run in a browser (no wasm backend, browser owns TLS),
  and the engines don't send CORS headers, so client-side scraping is
  impossible; the static-page + REST API split is the correct
  architecture.
- **Release workflow**: `.github/workflows/release.yml` - push a `v*`
  tag to build `ms`/`metasearch-api`/`metasearch-mcp` for linux
  x86_64/aarch64, windows x86_64, macos aarch64 plus the Python wheel,
  and publish a GitHub Release with sha256 checksums. Documented in
  docs/releasing.md.
- **Bug fixed**: `ms serve --no-mcp` (and `--no-rest`) exited
  immediately - the disabled listener completed instantly and
  `tokio::select!` returned. Now joined with `futures::try_join` so the
  enabled listener keeps running. Found by the smoke test.
- `cargo fmt`, `cargo clippy --workspace --all-targets` (zero warnings),
  `cargo test --workspace` (64 tests), end-to-end smoke: page, css, js,
  engines?category, extract, test, search endpoints all live.
