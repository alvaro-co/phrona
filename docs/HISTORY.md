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

## Final pass

- `cargo fmt`, `cargo clippy --all-targets` (library, API, MCP: zero
  warnings), `cargo test --workspace` (29 tests, offline via fixtures),
  `cargo build --release`.
- End-to-end smoke: REST API on a scratch port, MCP JSON-RPC session,
  Python wheel in a fresh venv.
