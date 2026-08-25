# Upstream sources

Phrona borrows approach, endpoints and code patterns from several open
source projects. Pinned commits are recorded in
`scripts/upstream-refs.txt`; the `upstream-watch` GitHub workflow
(and `scripts/watch_upstream.sh` locally) reports when any upstream moves
past its pin, so nothing breaks silently.

> Pins were corrected to the current upstream tips on 2026-08-10: the
> original pins for ddgs and websurfx were swapped, and 4get, searxng,
> primp, wreq, wreq-util and mcp-4get had moved.

## 4get (https://git.lolcat.ca/lolcat/4get) @ ba7ee6e

A minimal metasearch web service in JavaScript. The architectural blueprint:
each engine as a small self-contained module, scraping Google/Bing/DDG/Yahoo
HTML and the Qwant API. Borrowed from it:

- the idea of the DuckDuckGo **vqd token dance** (fetch
  `duckduckgo.com/?vqd=...`, use token to sign `/html/` and JSON endpoints);
- per-engine anti-bot workarounds (Startpage session token, `a=-1` params);
- the pragmatic approach of parsing engine HTML without a browser.

Monitor: when 4get changes its engine modules, the endpoints in
`crates/phrona/src/engines/` are likely to break too.

## SearXNG (https://github.com/searxng/searxng) @ 0a118066

The reference metasearch engine. Borrowed from it:

- the **SearXNG engine architecture**: `name`, `category`, `SearchEngine`-like
  interface (mirrored in our `engine::Engine` trait), per-engine modules in
  one directory, options struct (safesearch/region/language/time_range);
- **result merging and ranking**: URL normalization and dedup key (tracking
  parameter stripping) and the idea of ranking by cross-engine agreement
  (`rank::rank` implements agreement + position + text-match scoring);
- **answer extraction**: the notion of one-shot "answer" engines
  (SearXNG's `answerers`) -> our `grokipedia` answer engine.

Monitor: SearXNG engine changes (especially the Google/Bing/DDG parsers)
signal when scraping targets change.

## ddgs (https://github.com/deedy5/ddgs) @ a12929a

Python library for the DuckDuckGo backend. Borrowed from it:

- the exact DDG **JSON endpoints**: `/i.js` (images), `/news.js`,
  `/v.js` (videos), `/ac/` (suggestions) and their `vqd` signing;
- response field names and pagination (`s` cursor, `l` params).

Monitor: this is the live, battle-tested reference for all DDG endpoints;
changes here mean our DDG engines need updates.

Drift notes (2026-08-23 audit):

- ddgs has grown into a full multi-engine metasearch (text/images/news/
  videos/books with bing/brave/google/mojeek/startpage/yahoo/yandex/
  wikipedia/grokipedia backends). Its per-engine implementations are now
  a useful second reference for our own engine set; the DDG images
  engine still uses the identical `duckduckgo.com/?q=` vqd dance we do.
- Its grokipedia backend matches ours (`/api/typeahead`, `results[0]`,
  answer from `snippet`) - good sign that endpoint is still canonical.

## Websurfx (https://github.com/neon-mmd/websurfx) @ 09582be

Rust metasearch engine. Borrowed from it:

- the choice of **wreq** for HTTP impersonation (see below) and the overall
  "Rust-first, no browser" stance;
- result model ideas (dedup by URL after normalization).

## primp (https://github.com/deedy5/primp) @ 80cb5f3

Rust crate for command-line internet protocols. Its **`cookies` +
redirects + browser impersonation** primitives are the base of our
`HttpClient`.

## wreq (https://github.com/0x676e67/wreq) @ aa0063a, wreq-util @ 501b142

The HTTP/2 stack itself: TLS fingerprint spoofing with `Profile`/`Emulation`
(Chrome 100-149, Firefox 139-148, Safari 26, Edge 148, Opera 131, OkHttp),
HTTP/2 upgrade, connection pooling. This is the core anti-bot technology in
the project; upstream API changes require a coordinated update of
`HttpClient` (`crates/phrona/src/client.rs`).

## mcp-4get (https://github.com/yshalsager/mcp-4get) @ dd59cb7

A reference MCP server in TypeScript wrapping a metasearch API. Borrowed
from it:

- the **tool compartmentalization**: web/images/news/videos/books search
  as separate tools, plus suggestions and page fetch;
- the grounding/RAG pattern (search + extract best page + return excerpt).

Monitor: as MCP evolves (protocol versions, tool annotations), check this
project for the minimal accepted surface.

## How to monitor

```bash
./scripts/watch_upstream.sh          # report only (exit 0 = no drift)
./scripts/watch_upstream.sh --json   # machine-readable for automation
```

On GitHub, the `upstream-watch` workflow runs this weekly and opens an
issue when drift is detected. When drift is reported: diff the affected
repo against its pin, update any engine implementation that broke,
re-verify with the fixture workflow, then bump the pin in
`scripts/upstream-refs.txt`:

```bash
cargo run -p phrona --bin fetch_fixtures -- bing
cargo run -p phrona --bin dbg_parse -- bing
```

## Upstream drift observed 2026-08-23 (live audit)

What changed upstream since the last verification, and how phrona
reacted:

| Change | Reaction |
| --- | --- |
| Startpage fronted its SERP with **Anubis v1.25** proof-of-work | implemented natively (`engines/anubis.rs`); difficulty = leading zero hex digits of `SHA-256(randomData ++ nonce)` per the served worker JS |
| Google images async payload moved `original_image` to the item level and serves `application/json` | parser + media-type expectation fixed; engine live again |
| Yandex site-search markup renamed the result class to `b-serp-item` | selector fixed |
| Bing news infinite-scroll `first` semantics clarified (page N starts at `(N-1)*10+1`) | off-by-one fixed |
| Anna's Archive added ddos-guard/CHEQ interstitials on all mirrors | mirror list refreshed, failure classified as `UpstreamUnavailable`; engine needs residential proxies for now |
| Qwant/Mojeek/Google-web tightened datacenter-IP enforcement | documented as IP-blocked; parsers unchanged and fixture-tested |

Re-run this audit after any drift report by fetching each engine once
(`phrona test --json`) and comparing against the status table in
[engines.md](engines.md).
