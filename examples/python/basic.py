"""Basic usage of the phrona Python bindings.

Requires the wheel:  uv build && uv pip install dist/phrona-*.whl
Run:  python basic.py [query]
"""

import sys
import phrona

query = sys.argv[1] if len(sys.argv) > 1 else "rust programming"

print("version:", phrona.version())
print("engines:", ", ".join(phrona.engines("web")["web"]))

resp = phrona.search(query, max_results=5)
print(f"\nquery: {resp['query']} | category: {resp['category']} | "
      f"total: {resp['total']} | elapsed: {resp['elapsed_ms']} ms")

if resp.get("answer"):
    print("answer:", resp["answer"][:200])

for r in resp["results"]:
    print(f"\n{r['position']}. [{r['type']}] {r['title']}")
    print(f"   {r['url']}")
    print(f"   engines: {', '.join(r['engines'])}")

print("\nper-engine report:")
for e in resp["engines"]:
    print(f"  {e['name']:<16} {e['status']:<6} {e['results']} results")
