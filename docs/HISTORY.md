### perfection pass: robustness, dead code, staleness

- Orchestrator: blank queries rejected before any network; page
  normalized to >= 1 at the choke point (12 engine underflows gone);
  harvests run concurrently and bounded (180s each, always counted
  toward refresh spacing so a hanging browser cannot re-hang every
  search); `PHRONA_AUTO_BOOTSTRAP=1` honored on the config path too;
  new `with_concurrency` / `with_cache_ttl` builders (the CLI now
  applies both from config instead of ignoring them).
- Bootstrap: cached browser reused offline without a version fetch
  (numeric version ordering); runtime-nesting-safe download driver
  (works from current-thread runtimes); executable-bit `which`;
  CDP ping/pong; empty `PHRONA_CONFIG_PATH` treated as unset.
- Transport: mid-stream body errors keep timeout/network
  classification; 64-bit content-length compare; DDG vertical helper
  simplified; "a day ago" dates parse; annas all-empty reports schema
  instead of phantom 503; GitHub pins the API version and honors
  `Retry-After`/`x-ratelimit-reset`; archive.org yields `published`;
  arXiv lock is now `parking_lot`.
- Builders: `HttpClientBuilder::cookies(bool)` (the field was dead);
  `Category::NAMES`/`list_str` and `Profile::ALL_NAMES` end stale
  "expected one of" lists everywhere; `SearchResponse` gains
  `images()/news()/videos()/books()` accessors; `ExtractedPage`
  derives `Serialize`.
- Parsers: hoisted per-loop selectors (google, bing_news, brave,
  extract); bing reuses `time_param`; yahoo single selector;
  `wikipedia_lang` falls back instead of `unreachable!`; suggest
  parsers deduplicated; crypto documented with stack pads; ALTCHA
  boundary renamed off the old project name.
- Surfaces: `/v1/test` and `phrona test` probe concurrently;
  `/health` derives counts from the registry; frontend serves embedded
  assets zero-copy; MCP builds one service for all TCP connections;
  zero-limit clamps cannot panic (`clamp` with min > max);
  CLI extract clamps `max_chars`; Python clamps timeout (NaN-safe)
  and lists engines without building a client; manual `bootstrap`
  bounded with a timeout.
- Tooling: `fetch_fixtures` covers all 29 engines (github/arxiv/
  archive_org added), validates via real parsers, never aborts on one
  failure; `dbg_parse` covers every engine.
- Docs/assets: every stale count fixed (26->29, 5->8, nine->twelve),
  brave bullet rewritten for the SSR parser, fixture stats 25/29,
  API field lists corrected (`info`, `source`, `image_url`),
  web console lists all categories, `search_sync` semantics fixed.

## (2026-09-04) v0.3.0

- 29 engines across 8 categories (`github`/`code`, `arxiv`/`papers`,
  `archive_org`/`archives` are new); 199 offline tests green, zero
  clippy warnings; binaries + Python wheels publish from the `v0.3.0`
  tag via the release workflow.
- `quick-xml` 0.38.4 -> 0.41.0 (two HIGH advisories in the old
  version: quadratic attribute scan, unbounded `NsReader` allocation;
  the arXiv parser adapts to the new `BytesText` API and now decodes
  XML entities for display).
- The three sections below detail the release: new engines, the audit
  pass, and the perfection pass.

### new engines: github, arxiv, archive_org

- Three new engines behind three new categories: `github` (`code`,
  public repo search - code search needs auth and stays out), `arxiv`
  (`papers`, Atom feed), `archive_org` (`archives`, advancedsearch
  JSON). 29 engines across 8 categories.
- New categories reuse wire shapes (code/archives render as web
  results, papers as book results), so no consumer breaks; MCP gains
  `code_search` / `papers_search` / `archives_search`, the web console
  gains three chips, `/health` reports per-category counts.
- Upstream etiquette built in: arXiv requests throttle to one per 3s
  process-wide; GitHub quota exhaustion reports rate-limited, never
  blocked. New `quick-xml` dependency (pure Rust, no system packages).
- Fixtures captured live for all three; offline parser tests included.

### audit pass: orchestrator, classification, parsers

- Orchestrator: retry successes now count toward `any_ok` (previously an
  all-fail scrape plus a successful session retry still errored);
  engine waits and suggestions are deadline-bounded; reports sort by
  name; semaphore shutdown surfaces as an error instead of panicking;
  refresh candidacy keys off typed `ErrorKind`, not label strings.
- Error taxonomy: SSRF redirect refusals keep their classification
  through the transport (fixed-context recovery); I/O errors keep a
  static reason instead of discarding it.
- Dead config wired up: `search.cache_ttl_secs` drives the token caches;
  `security.block_private_ips` gates the SSRF guard (default on);
  env overrides apply once and atomically; CLI search/extract now
  honors the configured domain policy (previously open).
- Data fixes: brave_images results survive dedup (empty `url` dropped
  them all); DDG videos prefer the watch page and parse `1.2M` views;
  google_images shares the google session and tolerates trailing HTML;
  yandex anti-bot pages report blocked; yahoo drops the suggestions
  box; qwant/suggest locale and header cleanups.
- Hardening: PoW solvers and the curl fallback run off async workers;
  anubis refuses absurd difficulties; browser download streams to disk
  with a size cap and a strict version check; cookie cache writes are
  serialized; per-engine settle markers (qwant no longer idles 45s).
- Docs resynced (CLI/API/MCP/architecture/examples/compose), release
  workflow cross-compiles per target, `make lint` denies warnings.
  187 tests green, zero warnings/clippy.

## (2026-08-25) v0.2.0 review pass

- Dependency PRs resolved: base64 0.23, rmcp 3.1.4, wreq rc.31 (live-
  verified against real engines), rand 0.10 with the `RngExt` migration.
- Harvester correctness: session jar is re-read after the settle phase;
  a visit that never produces the marker cookie now fails instead of
  caching an unusable half-session; restarts honour per-engine refresh
  spacing via cached session ages; config pins always win over cache.
- Opt-in surface: `--auto-bootstrap` flag, `PHRONA_AUTO_BOOTSTRAP` env
  (library + CLI), config key - default off everywhere.
- Housekeeping: unused sha1 dependency dropped, debug scaffolding
  removed, zero warnings/clippy, 183 tests green.

# HISTORY

A log of how Phrona was built, commit by commit.

## (2026-08-25) session hardening: opt-in browser bootstrap, qwant pure-HTTP

- **Browser use is now strictly opt-in.** `engines.auto_bootstrap`
  defaults to *false*: phrona never launches a browser unless the
  operator enables it in config (`engines.auto_bootstrap: true`), via
  `PHRONA_ENGINES_AUTO_BOOTSTRAP=1`, or runs `phrona bootstrap <engine>`
  explicitly. Cached cookies keep working either way - warm start loads
  them without any browser involvement.
- **qwant is pure HTTP.** The engine consumes an operator-provided or
  cached session cookie but never spawns anything itself; the
  experimental in-browser fetch path was removed.
- Debug scaffolding removed (body dumps, per-engine trace prints);
  warnings at zero across the workspace.
- Documentation reviewed end to end to describe behaviour and
  configuration without detailing internal flows.

## (2026-08-25) self-contained headless bootstrap

The cookie harvester is pure Rust (a minimal CDP-over-WebSocket client,
no automation frameworks) and needs nothing from the environment:

- Runs the browser fully headless with a consistent session identity;
  the earlier virtual-display workaround is gone. Harvests take ~10-50 s
  per engine and are rate-limited (minutes between attempts).
- When no Chromium-family browser is installed, phrona downloads the
  official `chrome-headless-shell` build into the user cache directory
  on first use (~95 MB once, reused afterwards). No packages, no
  privileges; works on bare servers and containers.
  Env knobs: `PHRONA_NO_DOWNLOAD=1`, `PHRONA_BROWSER=/path`,
  `PHRONA_CACHE_DIR`.
- Verified on a simulated bare server (empty PATH): download -> harvest
  -> live searches for google and anna's archive.

## (2026-08-25) native silent bootstrap + dual transport

- **`bootstrap.rs`**: silent orchestrator bypass - when a seeded engine
  reports Blocked/NetworkFailure and auto-bootstrap is enabled, phrona
  harvests a fresh session (rate-limited), stores it in
  `phrona.cookies.json` next to the config, and re-runs just those
  engines once. Warm start: every client loads the cache first, so
  restarts reuse sessions without any browsing.
- **annas_archive transport fallback**: some upstreams treat HTTP
  clients differently, so after a failed mirror fetch the engine retries
  through system curl with the same cookies (capped attempts).
- **CLI**: `phrona bootstrap [engines...]`; global `--cookie
  engine=header`; config `engines.bootstrap_cookies`.
- Reality check: upstreams enforce reputation windows after heavy use -
  expect occasional blocks that clear themselves; matrix shows 20-26 of
  26 engines OK depending on the hour.

## (2026-08-23) engines revived: google + anna's archive

- **google web**: works with a real-browser session cookie
  (`__Secure-ENID`) replayed over plain HTTP. False-block bug fixed:
  healthy SERPs legitimately embed relay-page fragments as preload
  handlers; block markers tightened to the two specific interstitial
  phrases (+ regression test).
- **annas_archive**: mirror list gained `.gl` (first, matching the
  harvester seed); observed clearance lifetimes are short, so sessions
  are re-harvested rather than assumed fresh. Book search verified live
  (50 results).
- qwant pre-wired for an operator-provided session cookie.

## (production hardening)

- **Structured, allocation-free error system** (`error.rs`): `Error {
  scope, kind, engine, http_status, message }`. Scopes `Egress |
  Provider | Schema | Query | Internal`; kinds `RateLimited { retry_after }`,
  `Blocked(Cloudflare | Captcha | IpBan | BotDetection)`,
  `MalformedPayload { context }`, `UpstreamUnavailable { status }`,
  `Timeout`, `NetworkFailure`, `InvalidQuery { context }`,
  `AllProvidersFailed`. Constructor helpers plus typed
  `From<wreq::Error>` (using `is_timeout()`/`is_connect()`/... instead of
  string sniffing) and `From<io::Error>`. The string-based timeout sniffing
  in `client.rs` (`map_err`) was deleted.
- **Classifier upgraded to header signals** (`util.rs`): `cf-mitigated` /
  `cf-challenge` / `cf-ray`+403 → `Blocked(Cloudflare)`, `x-datadome` →
  `Blocked(Captcha)`, `429` → `RateLimited` honoring `Retry-After`, 403 →
  `Blocked(BotDetection)`, other non-2xx → `UpstreamUnavailable`; a 2xx
  with contradicting `Content-Type` → `MalformedPayload` (schema), not a
  rate limit. `parse_json_body` failures → `MalformedPayload`. DDG vqd /
  Startpage sc token failures → `Blocked(BotDetection)`. Still no body
  phrasing anywhere.
- **Streaming orchestrator** (`search.rs`): `FuturesUnordered` under one
  adaptive deadline (`SearchOptions.timeout`), early exit when the merged
  set reaches `max_results` (remaining in-flight futures cancelled), and
  the all-empty-is-an-error bug fixed: an error (`AllProvidersFailed`)
  only when every engine failed; otherwise empty results are honest.
- **`EngineReport`** gains `scope`/`kind` labels (`#[serde(default)]`) for
  failed engines; `Error::NoResults` removed.
- **REST API** maps error scope to HTTP status: 400 (Query), 429
  (RateLimited), 500 (Internal), 502 (Egress/Schema), 503 (Provider).

## c53cf7a - chore: scaffold workspace with wreq-based impersonating HTTP client

- Workspace `Phrona` with a single library crate `phrona`.
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
`crates/phrona/src/engines/*.rs`, driven by the fixture loop:
`fetch_fixtures <engine>` -> `dbg_parse <engine>` -> fix parser ->
commit. 26 engines across web/images/news/videos/books plus 7
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
(anomaly page). These are *environmental*, not parser bugs. Responses are
classified from HTTP semantics alone (status + `Content-Type`), and
`fetch_fixtures` only keeps captures that parse to results, so fixtures
never get polluted and parser tests skip non-SERP captures gracefully.

## 1a0857d - engines: fix bing_videos noscript parsing, tolerant tests for bot-blocked fixtures, clippy cleanup

- `regex_strip_noscript` fix for bing videos (see above).
- All `parse_fixture` tests consult the capture metadata (`fixture_parses`);
  `fetch_fixtures` validates captures by parsing them and never overwrites
  good fixtures with block pages.
- `brave_b64_decode` handles standard base64.
- Clippy cleanup across engines.

## 589953f - fetch_fixtures: fix Job future lifetime for non-static engine requests

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

## 683f068 - api: axum REST server with search, suggest, engines, health, Tavily-compatible /search, AI grounding

- `phrona-api` (axum 0.8 + tower-http):
  - `GET /` (frontend), `/health`, `/v1/engines`, `/v1/search`,
    `/v1/suggest`, `GET|POST /v1/grounding`.
  - `POST /search` + `POST /v1/tavily`: Tavily-compatible endpoint
    (search_depth, topic, days, include_images/answer/raw_content,
    include/exclude_domains) so Tavily clients work unchanged.
  - Optional auth via `PHRONA_API_KEY` (header/query/body); permissive
    CORS; tracing.
  - Decision: serve the frontend as **static files from the repo dir**
    (no embedding, no build step) - `frontend.rs` maps known asset
    paths, everything else falls back to the app shell.

## 2ffd027 - mcp: Model Context Protocol stdio server exposing phrona tools to AI agents

- `phrona-mcp` (rmcp 3.1, features `server` + `transport-io`,
  schemars, anyhow): 9 tools - web_search, image_search, news_search,
  video_search, book_search, suggest, fetch_page, search_grounded,
  list_engines.
- Verified by hand over JSON-RPC stdio: `initialize` (protocol
  2025-11-25), `tools/list`, `tools/call` returning real live results.
- `search_grounded` implements the RAG pattern: search, score, fetch the
  best page, return a verbatim excerpt + ranked sources.

## c676291 - python: pyo3 bindings with setuptools-rust wheel packaging (uv build verified)

- `phrona-python`: pyo3 0.24; cdylib named `phrona`; the
  internal Rust crate is aliased `phrona` to avoid a name
  collision on the produced artifacts.
- `Client` pyclass (profile + timeout, full SearchOptions kwargs) plus
  module-level `search`/`suggest`/`extract`/`engines`/`version`;
  `json_to_py` returns native Python dicts/lists.
- `ExtractedPage` got a manual `Serialize` impl so it can flow through
  the same JSON conversion path.
- Wheel packaging: workspace `pyproject.toml` with setuptools-rust,
  `[[tool.setuptools-rust.ext-modules]]` keyed by target name; verified
  with `uv build` -> `dist/phrona-*.whl`
  -> fresh venv install -> all functions called successfully.
- Verified quirk: pyo3 0.24 segfaults on CPython 3.14; Python <= 3.13
  only.

## 68910b0 - frontend: Material 3 style static SPA served by the API server

- `frontend/`: `index.html` + `style.css` (Material 3 tokens, light/dark
  themes, chips, cards, responsive image grid) + `app.js` (debounced
  suggestions, category/engine chips, region/language/time-range/max
  results controls, answer banner, error banner, theme persistence).
- API fallback fixed to serve assets by request URI path (the axum
  `Path<Vec<String>>` extractor receives an empty vec in fallback
  handlers - read `req.uri().path()` instead).

## 17683e2 - answer: grokipedia emits an answer marker; Tavily include_answer queries the answer engine

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

## 7dbde80 - tests: 35 new offline tests (64 total); five real bugs fixed

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

- **phrona-cli (`phrona`)** - one binary for every surface: `search`,
  `suggest`, `extract`, `ground`, `engines`, `test` (live availability
  probe), `serve` (REST + MCP-over-TCP in one process), `mcp` (stdio),
  `completions`. Generated completions via clap_complete; `--json`
  everywhere; the merged response carries the per-engine report.
- **`crates/phrona-api/src/lib.rs`** and
  **`crates/phrona-mcp/src/lib.rs`** extracted so both servers are
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
  `/v1/test` (the `phrona test` availability probe) so every CLI capability
  is reachable over HTTP.
- **CLI completeness**: `phrona extract` takes multiple URLs (parallel via
  `extract_many`); `phrona ground` accepts the full option set (category,
  region, language, time range, safesearch, filters, page); `phrona test`
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
  tag to build `phrona`/`phrona-api`/`phrona-mcp` for linux
  x86_64/aarch64, windows x86_64, macos aarch64 plus the Python wheel,
  and publish a GitHub Release with sha256 checksums. Documented in
  docs/releasing.md.
- **Bug fixed**: `phrona serve --no-mcp` (and `--no-rest`) exited
  immediately - the disabled listener completed instantly and
  `tokio::select!` returned. Now joined with `futures::try_join` so the
  enabled listener keeps running. Found by the smoke test.
- **Auth**: Bearer auth was documented but not implemented - the API now
  accepts `Authorization: Bearer <key>` everywhere, via one shared
  helper (`api_key_from_headers`), instead of three ad-hoc copies.
- **License**: AGPL-3.0 (LICENSE); Cargo.toml/pyproject.toml/README
  metadata aligned.
- `cargo fmt`, `cargo clippy --workspace --all-targets` (zero warnings),
  `cargo test --workspace` (64 tests), end-to-end smoke: page, css, js,
  engines?category, extract, test, search endpoints all live.
