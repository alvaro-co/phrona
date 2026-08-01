# Upstream sources

MetaSearchRS borrows approach, endpoints and code patterns from several open
source projects. Local clones for reference and diffing live in
`/tmp/opencode/repos/`; each project pins to the commit noted below.

## 4get (https://github.com/4get/4get) @ 8e7fe83

A minimal metasearch web service in JavaScript. The architectural blueprint:
each engine as a small self-contained module, scraping Google/Bing/DDG/Yahoo
HTML and the Qwant API. Borrowed from it:

- the idea of the DuckDuckGo **vqd token dance** (fetch
  `duckduckgo.com/?vqd=...`, use token to sign `/html/` and JSON endpoints);
- per-engine anti-bot workarounds (Startpage session token, `a=-1` params);
- the pragmatic approach of parsing engine HTML without a browser.

Monitor: when 4get changes its engine modules, the endpoints in
`crates/metasearch/src/engines/` are likely to break too.

## SearXNG (https://github.com/searxng/searxng) @ 8892414

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

## ddgs (https://github.com/ddh4t/ddgs) @ 09582be

Python library for the DuckDuckGo backend. Borrowed from it:

- the exact DDG **JSON endpoints**: `/i.js` (images), `/news.js`,
  `/v.js` (videos), `/ac/` (suggestions) and their `vqd` signing;
- response field names and pagination (`s` cursor, `l` params).

Monitor: this is the live, battle-tested reference for all DDG endpoints;
changes here mean our DDG engines need updates.

## Websurfx (https://github.com/websurfx/websurfx) @ a12929a

Rust metasearch engine. Borrowed from it:

- the choice of **wreq** for HTTP impersonation (see below) and the overall
  "Rust-first, no browser" stance;
- result model ideas (dedup by URL after normalization).

## primp (https://github.com/EricCrosson/primp) @ fd1f724

Rust crate for command-line internet protocols. Its **`cookies` +
redirects + browser impersonation** primitives are the base of our
`HttpClient`.

## wreq (https://github.com/daniel-lxs/wreq) @ 80cb5f3, wreq-util @ dd59cb7

The HTTP/2 stack itself: TLS fingerprint spoofing with `Profile`/`Emulation`
(Chrome 100-149, Firefox 139-148, Safari 26, Edge 148, Opera 131, OkHttp),
HTTP/2 upgrade, connection pooling. This is the core anti-bot technology in
the project; upstream API changes require a coordinated update of
`HttpClient` (`crates/metasearch/src/client.rs`).

## mcp-4get (https://github.com/4get/mcp-4get) @ 8e7fe83

A reference MCP server in TypeScript wrapping a metasearch API. Borrowed
from it:

- the **tool compartmentalization**: web/images/news/videos/books search
  as separate tools, plus suggestions and page fetch;
- the grounding/RAG pattern (search + extract best page + return excerpt).

Monitor: as MCP evolves (protocol versions, tool annotations), check this
project for the minimal accepted surface.

## How to monitor

```bash
for r in /tmp/opencode/repos/*/; do echo "== $r"; git -C "$r" log --oneline -3 origin/HEAD 2>/dev/null; done
```

Before updating any engine implementation, diff the relevant files against
the corresponding clone and check the live endpoint with `dbg_parse`:

```bash
cargo run -p metasearch --bin fetch_fixtures -- bing
cargo run -p metasearch --bin dbg_parse -- bing
```
