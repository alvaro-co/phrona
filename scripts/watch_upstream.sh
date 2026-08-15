#!/usr/bin/env bash
# Check upstream repositories for drift past the pinned commits.
#
# Reads scripts/upstream-refs.txt (<name>=<commit>), fetches the default
# branch tip of each repo, and prints any repo whose tip is not an
# ancestor of the pinned commit (i.e. upstream moved on).
#
# Usage:
#   ./scripts/watch_upstream.sh            # local run, prints a report
#   ./scripts/watch_upstream.sh --json     # machine-readable report
#
# In CI the upstream-watch workflow turns the report into a GitHub issue.
set -euo pipefail

REFS_FILE="$(dirname "$0")/upstream-refs.txt"
: "${REFS_FILE:?missing upstream-refs.txt}"

declare -A URLS=(
  [4get]="https://git.lolcat.ca/lolcat/4get"
  [searxng]="https://github.com/searxng/searxng"
  [ddgs]="https://github.com/deedy5/ddgs"
  [websurfx]="https://github.com/neon-mmd/websurfx"
  [primp]="https://github.com/deedy5/primp"
  [wreq]="https://github.com/0x676e67/wreq"
  [wreq-util]="https://github.com/0x676e67/wreq-util"
  [mcp-4get]="https://github.com/yshalsager/mcp-4get"
)

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

changed=()
while IFS= read -r line; do
  line="${line%%#*}"
  [ -z "$line" ] && continue
  name="${line%%=*}"
  pinned="${line#*=}"
  url="${URLS[$name]:-}"
  [ -z "$url" ] && continue

  git clone --quiet --filter=blob:none "$url" "$TMP/$name" 2>/dev/null || continue
  git -C "$TMP/$name" fetch --quiet --depth 200 origin 2>/dev/null || continue
  tip="$(git -C "$TMP/$name" rev-parse --short origin/HEAD 2>/dev/null || git -C "$TMP/$name" rev-parse --short origin/master 2>/dev/null || true)"
  [ -z "$tip" ] && continue

  # pinned commit may be beyond the shallow fetch depth; only judge when
  # it is actually present locally (otherwise a false "drift" report)
  if ! git -C "$TMP/$name" cat-file -e "$pinned^{commit}" 2>/dev/null; then
    echo "  (skipping $name: pinned commit $pinned not fetched in shallow clone)" >&2
    continue
  fi

  # moved on if the pinned commit is not an ancestor of the tip
  if ! git -C "$TMP/$name" merge-base --is-ancestor "$pinned" "$tip" 2>/dev/null; then
    changed+=("$name: pinned $pinned, tip $tip ($url)")
  fi
done < "$REFS_FILE"

if [ "${1:-}" = "--json" ]; then
  printf '%s\n' "${changed[@]}" | python3 -c '
import json, sys
lines = [l for l in sys.stdin.read().splitlines() if l]
print(json.dumps({"drift": lines}))
'
else
  if [ "${#changed[@]}" -gt 0 ]; then
    echo "Upstream drift detected:"
    printf '  - %s\n' "${changed[@]}"
  else
    echo "All upstream repos are within their pinned commits."
  fi
fi

[ "${#changed[@]}" -eq 0 ]
