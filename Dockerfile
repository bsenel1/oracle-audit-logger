FROM rust:1.91-bookworm AS builder

WORKDIR /app

COPY Cargo.toml Cargo.toml
COPY src src

RUN cargo build --release

FROM debian:bookworm-slim

WORKDIR /app

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/oracle_audit_file_collector /app/oracle_audit_file_collector

CMD ["/app/oracle_audit_file_collector"]
