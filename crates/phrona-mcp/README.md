# phrona-mcp

Model Context Protocol server exposing the
[phrona](https://crates.io/crates/phrona) metasearch engine to AI agents.
Works over stdio (JSON-RPC) and TCP, using rmcp.

## Tools

- `web_search`, `image_search`, `news_search`, `video_search`, `book_search`,
  `code_search`, `papers_search`, `archives_search`
- `suggest`
- `fetch_page` — page extraction
- `search_grounded` — grounded search for RAG
- `list_engines`

## Quick start

```bash
cargo add phrona-mcp
cargo run -p phrona-mcp          # stdio JSON-RPC
```

Point your MCP client (Claude Desktop, claude-code, Cursor...) at the
`phrona-mcp` binary. Via the CLI you can also serve MCP over TCP:

```bash
cargo install phrona-cli
phrona serve                     # REST 8080 + MCP-over-TCP 8081
```

See the [MCP reference](https://github.com/alvaro-co/phrona/blob/main/docs/mcp.md)
for tool parameters and examples.

## Related crates

- [phrona](https://crates.io/crates/phrona) — the engine library
- [phrona-api](https://crates.io/crates/phrona-api) — REST API server
- [phrona-cli](https://crates.io/crates/phrona-cli) — the `phrona` binary

## License

AGPL-3.0 — see the main repository for details.