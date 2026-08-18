# Build stage
FROM rust:alpine AS builder

RUN apk add --no-cache musl-dev make

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
    && addgroup -S fyrodb && adduser -S fyrodb -G fyrodb \
    && mkdir -p /data && chown fyrodb:fyrodb /data

WORKDIR /data

COPY --from=builder /app/target/release/fyro_db /usr/local/bin/fyro_db

# Configuration via environment variables
ENV FYRODB_PORT=8000
ENV FYRODB_WORKERS=0
ENV FYRODB_SHARDS=0
ENV FYRODB_MAX_KEYS=1000000
ENV FYRODB_MAX_CLIENTS=10000
ENV FYRODB_RDB_PATH=/data/fyrodb.rdb
ENV FYRODB_RDB_INTERVAL=300

EXPOSE 8000

USER fyrodb

CMD ["fyro_db"]
