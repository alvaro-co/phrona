# Engine reference

29 engines in 8 categories, all static stateless `Engine` instances
registered in `crates/phrona/src/engine.rs` (order = priority).

Status is the observed outcome from a datacenter IP (August 2026);
engines marked *IP-blocked* work from residential/clean IPs or through
proxies - their parsers are fixture-tested and degrade gracefully
(structured `Blocked`/`RateLimited` reports) when the network is
hostile. **22-29 of 29 engines return live results from this network** depending
on the hour (qwant
additionally requires running the harvester with `qwant`; brave/grokipedia
occasionally rate-limit or 502 transiently);
grokipedia additionally has intermittent server-side 502s (one retry is
built in).

| Name | Category | Endpoint | Status |
| --- | --- | --- | --- |
| duckduckgo | web | `https://html.duckduckgo.com/html/` | works |
| google | web | `https://www.google.com/search` | works with fresh bootstrap cookies (harvester) |
| bing | web | `https://www.bing.com/search` | works |
| brave | web | `https://search.brave.com/search` | works |
| mojeek | web | `https://www.mojeek.com/search` | works (ALTCHA PoW solved natively) |
| yahoo | web | `https://search.yahoo.com/search` | works |
| yandex | web | `https://yandex.com/search/site/` | works |
| startpage | web | `POST https://www.startpage.com/sp/search` | works (Anubis PoW solved natively; enforcement varies by IP) |
| qwant | web | `https://api.qwant.com/v3/search/web` | pure HTTP; accepts an operator-provided session cookie (`engines.bootstrap_cookies`) |
| marginalia | web | `https://old-search.marginalia.nu/search` | works (strict per-IP rate limit) |
| wikipedia | web | `https://en.wikipedia.org/w/api.php` | works |
| grokipedia | web | `https://grokipedia.com/api/typeahead` | works (server occasionally 502s) |
| duckduckgo_images | images | `https://duckduckgo.com/i.js` | works (browser headers + vqd-refresh retry; rate windows still occur) |
| bing_images | images | `https://www.bing.com/images/async` | works |
| brave_images | images | `https://search.brave.com/images` | works |
| startpage_images | images | `POST https://www.startpage.com/sp/search` | works (same Anubis flow) |
| mojeek_images | images | `https://www.mojeek.com/search?fmt=images` | works (same ALTCHA flow) |
| google_images | images | `https://www.google.com/search` (asearch=isch async JSON) | works |
| duckduckgo_news | news | `https://duckduckgo.com/news.js` | works (same hardening as images) |
| bing_news | news | `https://www.bing.com/news/infinitescrollajax` | works |
| yahoo_news | news | `https://news.search.yahoo.com/search` | works |
| brave_news | news | `https://search.brave.com/news` | works |
| duckduckgo_videos | videos | `https://duckduckgo.com/v.js` | works (same hardening as images) |
| bing_videos | videos | `https://www.bing.com/videos/asyncv2` | works |
| brave_videos | videos | `https://search.brave.com/videos` | works |
| annas_archive | books | `https://annas-archive.{gd,li,gl,se}/search` | works with a bootstrap session (auto-refreshed when enabled) |
| github | code | `https://api.github.com/search/repositories` | works (10 search req/min shared quota; exhaustion reports rate-limited) |
| arxiv | papers | `https://export.arxiv.org/api/query` | works (client-side 3s politeness throttle) |
| archive_org | archives | `https://archive.org/advancedsearch.php` | works |

Suggestion sources (7): duckduckgo, google, bing, brave, startpage,
qwant, wikipedia - all live-verified.

Availability probing: `phrona test` and `/v1/test` set
`SearchOptions::probe_all`, which runs every resolved engine to
completion and reports each one's outcome even for categories where
every engine failed. Normal searches keep the latency-oriented early
exit.

## Engine details and quirks

- **github** - public repository search only (code search needs
  authentication and is out of scope); the 10 req/min shared quota is
  reported as rate-limited, never as a block.
- **arxiv** - Atom feed parsed with `quick-xml`; a process-wide 3s gap
  between requests honors the upstream politeness rule.
- **archive_org** - `advancedsearch.php` JSON; metadata fields
  (identifier/title/description/mediatype/creator/date) arrive as strings
  *or* single-element lists; mediatype/creator/date fold into the
  description head.


- **bing** - HTML; `first=` paging; news via the
  `infinitescrollajax` JSON endpoint with `(page-1)*10+1` paging;
  videos from `asyncv2` with the result markup inside a `<noscript>`
  block (must be unwrapped before HTML parsing - see
  `regex_strip_noscript` in `bing_videos.rs`).
- **brave** - SSR HTML (`div[data-type="web"]` for web,
  `div.result-wrapper` for news/videos via the shared wrapper parser);
  safesearch / region travel in a shared cookie built by
  `brave::headers_for` (honored by every vertical including news);
  thumbnail URLs are base64-encoded (`/g:ce/...`), decoded with URL-safe
  and standard base64 (`brave_b64_decode`); page 2 via `?offset=`.
- **duckduckgo** - HTML endpoint with a plain GET (no `vqd` needed; the
  POST variant triggers the anomaly/bot page); images/news/videos use
  the JSON `vqd`-signed endpoints - the token is fetched from
  `duckduckgo.com/?q=...` once and cached in `EngineShared.vqd` per key
  (TTL 1 h). The vertical calls go through `util::fetch_ddg_vertical`,
  which sends duckduckgo.com's own frontend header set (`Referer`,
  `Sec-Fetch-*`, `Sec-GPC`) and, when the endpoint blocks or rate-limits,
  force-refreshes the token and retries exactly once. That took all three
  verticals from constant 403s to working from a datacenter IP; heavy
  automated bursts can still trip temporary rate windows.
- **google / google_images** - `/search` with `nfpr=1` (no autocorrect),
  consent cookies, modern selectors (`[jscontroller=SC7Lyd]`,
  `div[data-sncf]`) plus featured snippet as an answer. The images
  engine uses the `asearch=isch&async=_fmt:json,p:1,ijn:N` payload whose
  body is `)]}'`-prefixed JSON under `/ischj/metadata[]`; note that
  `original_image` sits at the item level next to `result`. Both engines
  recognize Google's HTTP-200 block pages ("detected unusual traffic",
  the enablejs relay) as `Blocked(Captcha)` when parsing yields nothing.
- **marginalia** - `old-search.marginalia.nu`; only queries of 3 words or
  fewer with plain alphanumeric text are issued (the site's own limit),
  otherwise the engine returns nothing. The site rate-limits hard when
  queried alongside other engines.
- **mojeek / mojeek_images** - HTML, images served through the same page.
  Unverified clients get an [ALTCHA](https://altcha.org) proof-of-work
  page; `engines/altcha.rs` solves it natively (PBKDF2/SHA-256 scan,
  ~0.3 s), submits it to `/captcha/verify` and rides the resulting
  `chllg` clearance cookie. Note: no manual Cookie headers are sent on
  these engines because an explicit Cookie value would evict the jar's
  clearance cookie.
- **qwant** - JSON API `api.qwant.com/v3` for results and suggestions;
  frequently gated on datacenter IPs without a session cookie.
- **yandex** - `search/site/` endpoint (site-restricted Yandex HTML, the
  only variant reliably parseable); organic results are
  `li.b-serp-item` with `a.b-serp-item__title-link` hrefs and
  `div.b-serp-item__text` descriptions; the standard `/search` may show
  the "there are no search results" anti-bot page - that page is also
  treated as a block page.
- **startpage** - POST search behind [Anubis](https://anubis.techaro.lol)
  proof-of-work: `engines/anubis.rs` extracts the challenge
  (`rules.difficulty` zero hex digits over `SHA-256(randomData ++ nonce)`),
  solves it, redeems it at
  `/.within.website/x/cmd/anubis/api/pass-challenge` and retries the POST
  on the same cookie jar. Unparseable challenges and difficulties above
  16 are rejected at extraction; difficulties above 8 are refused at
  solve time (16^d expected hashes would be a CPU-DoS, not a challenge);
  nonces past 2^28 give up (reported as blocked). The `sc` token dance is unchanged (GET homepage
  -> `input[name=sc]` -> cached in `EngineShared.sc`, TTL 1 h);
  suggestions come from `/suggestions`; images via the same POST flow
  with `cat=images`. Results are parsed from the embedded React payload
  (`React.createElement(UIStartpage.AppSerp*, {...})`), with an HTML
  fallback for the web variant.
- **wikipedia** - OpenSearch API plus a best-effort extract call for the
  description (a failure there still returns the article link).
- **grokipedia** - typeahead + `/page/` fetches; the typeahead snippet is
  returned as an **answer marker** (empty URL, capped at a sentence
  boundary) so responses carry a real `answer` field (used verbatim by
  `include_answer` in the Tavily endpoint and by the grounding endpoint),
  plus the page result itself.
- **yahoo / yahoo_news** - HTML, `_ylt/_ylu` path-token obfuscation for
  web; news has its own layout; relative dates are normalized to ISO-8601
  by the shared `util::normalize_date`.
- **annas_archive** - mirror failover over
  `annas-archive.gd` / `.li` / `.gl` / `.se`; mirrors gate plain HTTP
  clients behind browser sessions, so the engine pairs with the
  bootstrap session machinery. The parser itself is fixture-tested.

## Response classification

Every engine funnels its HTTP response through `util::check_response(engine,
&resp, MediaType)` in `engines/util.rs`, which classifies purely from HTTP
metadata (RFC 9110) - never from body phrasing:

- header-level anti-bot signals first: `cf-mitigated` / `cf-challenge` /
  `cf-ray`+403 → `Blocked(Cloudflare)`, `x-datadome` → `Blocked(Captcha)`;
- `429 Too Many Requests` → `RateLimited` (honoring the `Retry-After`
  header);
- `403 Forbidden` → `Blocked(BotDetection)`;
- any other non-2xx status → `UpstreamUnavailable`;
- a 2xx response whose `Content-Type` contradicts the expected media type
  (`MediaType::Html` or `MediaType::Json`) → `MalformedPayload` (a
  structural deviation, not a rate limit).

A 2xx response of the expected type is trusted as-is; whether it contains
results is the parser's job - an empty parse is an honest "no results", not
a hidden failure. The single exception, documented here deliberately: three
engines serve their block pages with HTTP 200 (Google CAPTCHA/relay,
Startpage anomaly pages, Yandex no-results page), so after a zero-result
parse they check a small list of known markers and report `Blocked`
instead. An honest empty SERP can never be misclassified because real
SERPs never contain those markers. JSON endpoints additionally run the
body through `util::parse_json_body`, which validates that the payload is
actually JSON (grammar check; failures classify as `MalformedPayload`),
and Qwant/DDG check the API `status` field.

## Fixtures

`crates/phrona/tests/fixtures/` holds one captured live page per engine
plus a `meta.json` sidecar recording each capture's `status`,
`content_type` and `parsed` verdict. Tests run fully offline and never hit
the network. As of this pass, 25 of 29 fixtures are verified-parseable
live captures; the four skipped ones are exactly the IP-blocked engines
(google_web, mojeek_web/images, qwant_web), whose tests assert graceful
behavior instead.

`fetch_fixtures` mirrors the engine implementations exactly (including
the Startpage Anubis solve and google_images' clean cookie jar - shared
Google cookies change the served payload variant), only keeps captures
that are 2xx + expected Content-Type + parser-extractable (content
validation - no marker-string sniffing), and re-validates kept fixtures
when a re-capture is blocked so a transient failure never flips a valid
fixture into a skipped one. Re-capture with:

```bash
cargo run -p phrona --bin fetch_fixtures [engine...]
```

`dbg_parse` prints parsed results for a fixture:

```bash
cargo run -p phrona --bin dbg_parse -- bing
```
