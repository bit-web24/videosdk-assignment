# Regionally Distributed WebSocket Communication System

A WebSocket server system where clients connect to their nearest regional server and communicate seamlessly with clients connected to other regions — built in Rust.

## High-Level Design

```
                        ┌──────────────────────────────────────┐
Client (alice)          │  Region A — us-east (port 3001)      │
    │                   │                                      │
    └──── WebSocket ───▶│  connections: { alice → channel }    │
                        │  presence:   { alice → us-east,      │
                        │               bob   → eu-west }      │
                        │                    │                 │
                        └────────────────────│─────────────────┘
                                             │ NATS pub/sub
                        ┌────────────────────│─────────────────┐
                        │  Region B — eu-west (port 3002)      │
                        │                    │                 │
                        │  connections: { bob → channel }      │
                        │  presence:   { alice → us-east,      │
                        │               bob   → eu-west }      │
    ┌──── WebSocket ───▶│                                      │
    │                   └──────────────────────────────────────┘
Client (bob)

Global Router (port 8080): GET /route?region=us-east → ws://localhost:3001/ws
```

### Message Flow

1. Alice sends `{ "to": "bob", "content": "hello" }` to her regional server (us-east).
2. us-east checks its local `connections` map — Bob is not here.
3. us-east checks its local `presence` map — Bob is in eu-west.
4. us-east publishes the message to NATS subject `messages.eu-west`.
5. eu-west's background subscriber receives it and delivers it to Bob's WebSocket channel.


---

## Components

| Service | Port | Description |
|---|---|---|
| `nats` | 4222 | NATS messaging server (inter-region transport) |
| `nats` monitor | 8222 | NATS HTTP monitoring dashboard |
| `region-a` | 3001 | WebSocket server — simulates us-east |
| `region-b` | 3002 | WebSocket server — simulates eu-west |
| `router` | 8080 | Global traffic router — maps region names to WS URLs |

---

## Setup

### Prerequisites

- [Docker](https://docs.docker.com/get-docker/) and [Docker Compose](https://docs.docker.com/compose/)
- [Rust](https://rustup.rs/) (for running the CLI client locally)

### Pull the NATS image

```bash
docker pull nats:2.14.3-alpine
```

---

## Starting the System

Build and start all services:

```bash
docker-compose up --build
```

Start in background:

```bash
docker-compose up --build -d
```

Check all services are healthy:

```bash
docker-compose ps
```

View logs for a specific region:

```bash
docker-compose logs -f region-a
docker-compose logs -f region-b
docker-compose logs -f router
```

Open the NATS monitoring dashboard in your browser:

```
http://localhost:8222
```

---

## Running the Clients

The CLI client asks the router which WebSocket server handles the requested region, connects to it, and lets you exchange messages in real time.

Build the client binary:

```bash
cargo build --bin client
```

Open **two separate terminals**:

**Terminal 1 — Alice connecting to us-east:**

```bash
cargo run --bin client -- --user-id=alice --region=us-east --to=bob
```

**Terminal 2 — Bob connecting to eu-west:**

```bash
cargo run --bin client -- --user-id=bob --region=eu-west --to=alice
```

Once both are connected, type a message in either terminal and press Enter. It will appear in the other terminal.

### Client flags

| Flag | Required | Default | Description |
|---|---|---|---|
| `--user-id` | Yes | — | Your user identity on the network |
| `--region` | Yes | — | Region to connect to (`us-east` or `eu-west`) |
| `--to` | Yes | — | User ID of the person you want to message |
| `--router` | No | `http://localhost:8080` | URL of the global traffic router |

### Same-region test

Both clients can connect to the same region — messages are delivered locally without touching NATS:

```bash
# Terminal 1
cargo run --bin client -- --user-id=alice --region=us-east --to=bob

# Terminal 2
cargo run --bin client -- --user-id=bob --region=us-east --to=alice
```

---

## Project Structure

```
src/
├── main.rs          Entry point for the regional WebSocket server
├── config.rs        Reads REGION_ID, PORT, NATS_URL from environment
├── state.rs         Shared AppState: connections map, presence map, NATS client
├── nats.rs          NATS wire types, publishers, and background subscriber tasks
├── ws.rs            WebSocket upgrade handler, message routing logic
└── bin/
    ├── router.rs    Global traffic router HTTP service
    └── client.rs    CLI client binary
```

---

## Tech Stack

| Crate | Purpose |
|---|---|
| `axum` | WebSocket server and HTTP routing |
| `tokio` | Async runtime |
| `async-nats` | NATS client for inter-region messaging |
| `dashmap` | Thread-safe in-memory maps for connections and presence |
| `serde` / `serde_json` | Message serialization |
| `uuid` | Auto-generate user IDs when none is provided |
| `tokio-tungstenite` | WebSocket client for the CLI binary |
| `reqwest` | HTTP client used by the CLI to query the router |
| `tracing` | Structured async logging |
| `dotenvy` | Load configuration from `.env` in local development |
| `thiserror` | Ergonomic error types |

---

## Stopping the System

```bash
docker-compose down
```
