# Build stage
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev

WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -rf src

COPY src ./src
RUN touch src/main.rs && cargo build --release

# Runtime stage
FROM alpine:latest

RUN apk add --no-cache ca-certificates \
    && addgroup -S flashdb && adduser -S flashdb -G flashdb \
    && mkdir -p /data && chown flashdb:flashdb /data

WORKDIR /data

COPY --from=builder /app/target/release/flash_db /usr/local/bin/flash_db

# Configuration via environment variables
ENV FLASHDB_PORT=8000
ENV FLASHDB_WORKERS=0
ENV FLASHDB_SHARDS=0
ENV FLASHDB_MAX_KEYS=1000000
ENV FLASHDB_MAX_CLIENTS=10000
ENV FLASHDB_RDB_PATH=/data/flashdb.rdb
ENV FLASHDB_RDB_INTERVAL=300

EXPOSE 8000

USER flashdb

CMD ["flash_db"]
