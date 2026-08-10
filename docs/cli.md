# CLI reference

`cargo run -p metasearch-cli -- ...` (binary name `ms`). A single entry
point to every library feature plus the full server: search, suggestions,
extraction, grounding, engine listing, availability probing, the REST API
and the MCP server.

## Global options

| Option | Meaning |
| --- | --- |
| `--json` | machine-readable output (all commands) |
| `--profile <name>` | browser impersonation: chrome, firefox, safari, edge, opera, okhttp, random |
| `--proxy <url>` | proxy URL, repeatable (pool) |
| `--timeout <sec>` | request timeout (default 20) |
| `-h`, `-V` | help, version |

## Commands

### ms search <query>

Full search with every option:

```bash
ms search "rust ownership" --max-results 10 --engines bing,brave,wikipedia
ms search "rust" --category news --time-range week --region us-en --json
ms search "rust" --category images --safesearch strict --max-results 20
```

Options: `--category web|images|news|videos|books`, `--engines <csv>`,
`--max-results`, `--safesearch off|moderate|strict`, `--region`, `--language`,
`--time-range day|week|month|year`, `--filters`, `--page`.

Text output shows the query summary, answer, suggestions, ranked results
with engine provenance, and the per-engine report (status, result count,
error). `--json` emits the same shape as `GET /v1/search`.

### ms suggest <query>

```bash
ms suggest rus --source bing,wikipedia
ms suggest rus --json                # all 7 sources
```

### ms extract <url> [url...]

One or more URLs, fetched and extracted in parallel. `--query` biases
the excerpt; `--max-chars` caps the text.

```bash
ms extract https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html \
  --max-chars 3000 --query ownership
ms extract https://example.com https://example.org --max-chars 800
```

### ms ground <query>

Grounded output for RAG: the answer (library answer engine verbatim when
present, otherwise an extractive summary) followed by ranked cited sources.
Accepts the full search option set: `--category`, `--engines`,
`--max-results`, `--region`, `--language`, `--time-range`, `--safesearch`,
`--filters`, `--page`.

```bash
ms ground "rust ownership"
ms ground "rust" --category news --time-range week --region us-en --max-results 5
```

### ms engines

```bash
ms engines                      # all categories
ms engines --category videos
```

### ms test

Availability probe across every category (or one, with `--category`).
Runs a real search per category and prints an availability matrix plus
per-engine status, result counts and errors. Useful for smoke-testing a
network, a proxy setup or a profile choice.

```bash
ms test --query "rust programming"
ms test --category web --max-results 8
ms test --category web --json
```

### ms serve

The full server in one process:

```bash
ms serve                                   # REST on 127.0.0.1:8080 + MCP on tcp 127.0.0.1:8081
ms serve --addr 0.0.0.0:9090 --mcp-addr 0.0.0.0:9091 --api-key secret
ms serve --no-mcp                          # REST only
ms serve --no-rest                         # MCP-over-TCP only
```

- REST API: identical to `metasearch-api` (frontend at `/`, `/health`,
  `/v1/*`, Tavily-compatible `/search`). See [docs/api.md](api.md).
- MCP over TCP: the same nine tools as the stdio server, framed as
  newline-delimited JSON-RPC 2.0 over a raw TCP socket. Clients that
  cannot use stdio (remote agents, containers) connect with any MCP
  client configured for a TCP transport.
- `--api-key` / `META_API_KEY` guards the REST API; the MCP listener is
  unauthenticated (bind it to localhost or a private network).

### ms mcp

Serve MCP over stdio only (the same contract as `metasearch-mcp`):
```bash
ms mcp
```

### ms completions <shell>

```bash
ms completions bash > ~/.bash_completion.d/ms
ms completions zsh > "$fpath[1]/_ms"
```

## Shell completions

Generated from the clap definition via `clap_complete` (bash, zsh, fish,
powershell, elvish).

## Exit codes

0 on success, 1 on search/extraction failure (e.g. all engines blocked),
2 on argument errors (clap default).

## JSON shapes

`search --json` and `ground --json` mirror `GET /v1/search`; `suggest
--json` mirrors `GET /v1/suggest` (all sources when no `--source`);
`engines --json` mirrors `GET /v1/engines`; `test --json` is a list of
per-category `{category, total, elapsed_ms, engines[]}` objects. All
shapes are documented in [docs/api.md](api.md).
