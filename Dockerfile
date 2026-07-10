FROM rust:1.96-slim AS builder

WORKDIR /app

# async-nats links against OpenSSL on Linux
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests, vendored dependencies, and cargo config.
# The vendor/ directory makes this build fully offline — no crates.io access needed.
COPY Cargo.toml Cargo.lock ./
COPY vendor ./vendor
COPY .cargo ./.cargo
COPY src ./src

RUN cargo build --release

FROM debian:bookworm-slim AS runtime

WORKDIR /app

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/videosdk-assignment ./server

EXPOSE 3000

CMD ["./server"]
