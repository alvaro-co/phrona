# Examples

Runnable implementations of every public surface, in Rust and Python.

## Layout

```text
examples/rust/       a workspace crate with four runnable binaries
examples/python/     scripts against the built wheel (see its README)
```

## Rust

The `phrona-examples` crate depends only on the core library
(no API/MCP layers), demonstrating the minimal integration path.

| Binary | Demonstrates |
| --- | --- |
| `basic` | sync (`search_sync`), async (`search`), reusable `SearchClient`, typed `web()` accessor, per-engine report |
| `suggest` | single-source and parallel all-source suggestions |
| `extract` | `extract` and `extract_many` with query-biased excerpts |
| `ground` | grounded RAG output: answer plus cited sources |

```bash
cargo run -p phrona-examples --bin basic -- "rust programming"
cargo run -p phrona-examples --bin suggest -- "rust"
cargo run -p phrona-examples --bin extract
cargo run -p phrona-examples --bin ground -- "rust ownership"
```

## Python

```bash
uv build
uv venv --python 3.12 /tmp/msenv
uv pip install --python /tmp/msenv/bin/python dist/phrona-0.1.0-*.whl
/tmp/msenv/bin/python examples/python/basic.py "rust programming"
/tmp/msenv/bin/python examples/python/client.py "rust ownership"
```

`basic.py` uses the module-level functions; `client.py` uses the reusable
`Client` class with suggestions, extraction and per-category engine lists.
Full details in `examples/python/README.md`.

## Using everything together

The same library powers every surface; the crates compose in one process:

- `phrona` - core (Rust or via `phrona` Python package)
- `phrona-api` - REST API, also embedded in `phrona serve`
- `phrona-mcp` - MCP stdio server, also embedded in `phrona serve`
  and `phrona mcp`
- `phrona-cli` (`phrona`) - all of the above plus search/suggest/extract/
  ground/test commands

Example: run `phrona serve`, then query it from Rust (`SearchClient`),
Python (`phrona`), HTTP (`curl`), or an MCP client pointed at
`tcp://127.0.0.1:8081` - one process, four interfaces.
