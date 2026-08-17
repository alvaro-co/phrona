# phrona-cli

Command-line interface to the [phrona](https://crates.io/crates/phrona)
metasearch engine. The `phrona` binary is the all-in-one entry point:
search, suggest, extract, grounding, engine listing, availability tests,
the full REST server, an MCP server (stdio or TCP) and shell completions.

## Install

```bash
cargo install phrona-cli
```

## Usage

```bash
phrona search "rust programming" --max-results 10
phrona suggest rus
phrona extract "https://example.com" --ground
phrona engines
phrona test                 # availability probe
phrona serve                # REST 8080 + MCP-over-TCP 8081
phrona mcp                  # MCP over stdio
phrona completions bash     # shell completions
```

See the [CLI reference](https://github.com/alvaro-co/phrona/blob/main/docs/cli.md)
for all commands and flags.

## Related crates

- [phrona](https://crates.io/crates/phrona) — the engine library
- [phrona-api](https://crates.io/crates/phrona-api) — REST API server
- [phrona-mcp](https://crates.io/crates/phrona-mcp) — MCP server

## License

AGPL-3.0 — see the main repository for details.