# ─── Stage 1: Builder ────────────────────────────────────────────────────────
FROM rust:1.87-slim AS builder

WORKDIR /app

# Install system dependencies needed for compilation
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    && rm -rf /var/lib/apt/lists/*

# Copy manifests first (layer caching — only re-downloads deps when Cargo files change)
COPY Cargo.toml Cargo.lock* ./

# Build a dummy main to cache dependencies
RUN mkdir src && echo "fn main() {}" > src/main.rs
RUN cargo build --release
RUN rm -f target/release/deps/videosdk_assignment*

# Now copy the real source code and build
COPY src ./src
RUN cargo build --release

# ─── Stage 2: Runtime ────────────────────────────────────────────────────────
FROM debian:bookworm-slim AS runtime

WORKDIR /app

# Install runtime dependencies (openssl, ca-certs for TLS)
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/*

# Copy the compiled binary from the builder stage
COPY --from=builder /app/target/release/videosdk-assignment ./server

# Expose the port (overridden per region via env)
EXPOSE 3000

CMD ["./server"]
