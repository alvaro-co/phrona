# Releasing

Versioned releases are published from GitHub Actions by pushing a version
tag. The `release` workflow builds binaries for Linux (x86_64 + aarch64),
Windows (x86_64) and macOS (aarch64), builds the Python wheel, and creates
a GitHub Release with all artifacts attached.

## How to release

```bash
# 1. Bump the version in Cargo.toml (workspace.package) and pyproject.toml
#    (also: add a HISTORY.md entry describing the release).

# 2. Commit and push:
git add -A && git commit -m "release: vX.Y.Z"
git push

# 3. Tag and push the tag:
git tag vX.Y.Z
git push origin vX.Y.Z
```

The tag name must start with `v` (e.g. `v0.1.0`, `v1.2.3`). Pushing it
triggers `.github/workflows/release.yml`.

## What the workflow produces

| Artifact | Contents |
| --- | --- |
| `metasearch-<target>.tar.gz` / `.zip` | release binaries `ms`, `metasearch-api`, `metasearch-mcp` (+ `.sha256`) |
| `metasearch-wheel.whl` | Python wheel (linux x86_64) |

`ms` embeds everything: run `ms serve` for the full server (REST +
MCP-over-TCP + web page with Search and Tools tabs), or use the
individual binaries.

## What CI checks before you release

`.github/workflows/ci.yml` runs on every push/PR: `cargo fmt --check`,
`cargo clippy --workspace --all-targets -D warnings`, `cargo test
--workspace` (offline fixture tests) and a release build. `make check`
runs the same locally.

## Post-release

- The `upstream-watch` workflow keeps tracking the 8 upstream projects;
  address any drift issues it opens before the next release.
- Update the `version` note in docs if the API surface changed
  (`/health` reports the crate version).
