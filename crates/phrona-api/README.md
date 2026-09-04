# phrona-api

REST API server for the [phrona](https://crates.io/crates/phrona) metasearch
engine, built with axum. Search, suggestions, page extraction, AI grounding,
engine availability probing and a Tavily-compatible endpoint — plus a
Material 3 web frontend served from the same process.

## Quick start

```bash
cargo add phrona-api
cargo run -p phrona-api          # listens on 127.0.0.1:8080
curl "localhost:8080/v1/search?q=rust&max_results=5"
```

## Endpoints

- `GET /v1/search` — metasearch (native parameters)
- `POST /search`, `POST /v1/tavily` — Tavily-compatible search
- `GET /v1/suggest` — suggestions
- `GET /v1/extract` — page extraction / AI grounding
- `GET /v1/engines` / `GET /v1/test` — engine listing and availability probing
- `GET /health` — health check
- `GET /` — the web frontend (served from `assets/`, editable without rebuild)

See the [API reference](https://github.com/alvaro-co/phrona/blob/main/docs/api.md)
for the full endpoint contract.

## Related crates

- [phrona](https://crates.io/crates/phrona) — the engine library
- [phrona-mcp](https://crates.io/crates/phrona-mcp) — MCP server for AI agents
- [phrona-cli](https://crates.io/crates/phrona-cli) — the `phrona` binary
  (`phrona serve` runs this server)

## License

AGPL-3.0 — see the main repository for details.