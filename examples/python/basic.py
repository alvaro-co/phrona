"""Basic usage of the metasearch Python bindings.

Requires the wheel:  uv build && uv pip install dist/metasearch-*.whl
Run:  python basic.py [query]
"""

import sys
import metasearch

query = sys.argv[1] if len(sys.argv) > 1 else "rust programming"

print("version:", metasearch.version())
print("engines:", ", ".join(metasearch.engines("web")))

resp = metasearch.search(query, max_results=5)
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
