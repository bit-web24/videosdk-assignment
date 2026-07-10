FROM rust:1.87-slim AS builder

WORKDIR /app

# async-nats links against OpenSSL on Linux
RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

# Copy manifests and pre-build dependencies so Docker can cache this layer.
# The real source is copied after, so dependency layers only rebuild when Cargo files change.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -f target/release/deps/videosdk_assignment*

COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim AS runtime

WORKDIR /app

RUN apt-get update && apt-get install -y ca-certificates libssl3 && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/videosdk-assignment ./server

EXPOSE 3000

CMD ["./server"]
