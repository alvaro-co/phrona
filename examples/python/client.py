"""Reusable Client, suggestions, extraction and grounded output.

Run:  python client.py [query]
"""

import sys

import phrona

query = sys.argv[1] if len(sys.argv) > 1 else "rust ownership"

client = phrona.Client(profile="chrome", timeout=20)

# Suggestions from a single source and from all sources.
print("suggest (bing):", phrona.suggest("rus", source="bing", region="us-en"))
all_sugg = phrona.suggest("rus")
for source, items in all_sugg["suggestions"].items():
    print(f"suggest ({source}): {items[:4]}")

# Search through the client with full options.
resp = client.search(
    query,
    engines=["bing", "brave", "wikipedia", "grokipedia"],
    max_results=6,
    safesearch="moderate",
)
print(f"\nanswer: {resp.get('answer') or '(none)'}")

# Page extraction (readable text for RAG).
page = client.extract(
    "https://doc.rust-lang.org/book/ch04-01-what-is-ownership.html",
    max_chars=800,
    query=query,
)
print(f"\nextract: {page['title']}")
print(f"  description: {page['description'][:120]}")
print(f"  text: {page['text'][:200]}...")

# Grounded output: best answer plus cited sources.
print("\nengines by category:")
for cat in ("web", "images", "news", "videos", "books", "code", "papers", "archives"):
    print(f"  {cat}: {len(phrona.engines(cat)[cat])} engines")
