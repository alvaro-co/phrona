# Multi-stage build: Rust release binary + frontend assets.
FROM rust:1.85-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --locked -p phrona-cli && \
    cp target/release/phrona /phrona

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /phrona /usr/local/bin/phrona
COPY --from=builder /build/frontend /usr/share/phrona/frontend
ENV PHRONA_ADDR=0.0.0.0:8080
ENV PHRONA_FRONTEND_DIR=/usr/share/phrona/frontend
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s \
    CMD curl -fsS http://127.0.0.1:8080/health || exit 1
ENTRYPOINT ["phrona", "serve", "--no-mcp"]
