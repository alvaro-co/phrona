.PHONY: all build fmt fmt-check lint test check release wheel examples serve-api serve-all clean

all: check

build:
	cargo build --workspace

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all --check

lint:
	cargo clippy --workspace --all-targets

test:
	cargo test --workspace

check: fmt-check lint test

release:
	cargo build --release -p metasearch-cli -p metasearch-api -p metasearch-mcp

wheel:
	uv build

examples: release
	cargo run -p metasearch-examples --bin basic -- "rust programming"
	cargo run -p metasearch-examples --bin suggest -- "rust"
	cargo run -p metasearch-examples --bin extract

serve-api:
	cargo run -p metasearch-cli -- serve --no-mcp

serve-all:
	cargo run -p metasearch-cli -- serve

clean:
	cargo clean
	rm -rf dist *.egg-info .venv
