# Multi-stage build: Rust release binary + frontend assets.
FROM rust:1.85-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --locked -p phrona-cli && \
    cp target/release/phrona /phrona

FROM debian:bookworm-slimRUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl && \
    useradd --create-home --shell /usr/sbin/nologin phonrona && \
    rm -rf /var/lib/apt/lists/*
COPY --from=builder /phrona /usr/local/bin/phrona
COPY --from=builder /build/frontend /usr/share/phrona/frontend
ENV PHRONA_ADDR=0.0.0.0:8080
ENV PHRONA_FRONTEND_DIR=/usr/share/phrona/frontend
EXPOSE 8080
# Run as an unprivileged user: the API never writes to the filesystem, so
# the least-privileged identity is the safest default.
USER phonrona
HEALTHCHECK --interval=30s --timeout=3s --retries=3 \
    CMD curl -f http://localhost:8080/health || exit 1
ENTRYPOINT ["/usr/local/bin/phrona", "serve"]