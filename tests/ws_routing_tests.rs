// Integration tests for WebSocket message routing logic in src/ws.rs.
// Tests exercise the full WebSocket handler using a real TCP listener and
// tokio-tungstenite client.
//
// A NATS server is started via Docker for the duration of the test suite
// (using a random host port) and torn down afterwards. If Docker is not
// available the tests are skipped gracefully.
//
// Run with: cargo test --test ws_routing_tests

use axum::{routing::get, Router};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{net::SocketAddr, time::Duration};
use tokio::net::TcpListener;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use videosdk_assignment::state::AppState;
use videosdk_assignment::ws::ws_handler;

// ── NATS test harness ────────────────────────────────────────────────────────

struct NatsServer {
    container_id: String,
    port: u16,
}

impl NatsServer {
    /// Start a NATS container on a random host port.
    /// Returns None if Docker is not available.
    async fn start() -> Option<Self> {
        // Check Docker is available
        let check = tokio::process::Command::new("docker")
            .arg("info")
            .output()
            .await
            .ok()?;
        if !check.status.success() {
            return None;
        }

        // Pick a free port
        let listener = TcpListener::bind("127.0.0.1:0").await.ok()?;
        let port = listener.local_addr().ok()?.port();
        drop(listener);

        let out = tokio::process::Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "-p",
                &format!("{}:4222", port),
                "nats:2.14.3-alpine",
            ])
            .output()
            .await
            .ok()?;

        if !out.status.success() {
            return None;
        }

        let container_id = String::from_utf8(out.stdout).ok()?.trim().to_string();

        // Wait until NATS is ready (up to 8 s)
        let url = format!("nats://127.0.0.1:{}", port);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
        loop {
            if tokio::time::Instant::now() > deadline {
                // Tear down and give up
                let _ = tokio::process::Command::new("docker")
                    .args(["stop", &container_id])
                    .output()
                    .await;
                return None;
            }
            if async_nats::connect(&url).await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Some(NatsServer { container_id, port })
    }

    fn url(&self) -> String {
        format!("nats://127.0.0.1:{}", self.port)
    }

    async fn stop(&self) {
        let _ = tokio::process::Command::new("docker")
            .args(["stop", &self.container_id])
            .output()
            .await;
    }

    async fn connect(&self) -> async_nats::Client {
        async_nats::connect(self.url())
            .await
            .expect("connect to test NATS")
    }
}

// ── App builder helpers ──────────────────────────────────────────────────────

fn build_ws_app(state: AppState) -> Router {
    Router::new()
        .route("/ws", get(ws_handler))
        .with_state(state)
}

async fn spawn_ws_server(state: AppState) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind random port");
    let addr: SocketAddr = listener.local_addr().expect("local_addr");
    let app = build_ws_app(state);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("ws server");
    });
    format!("ws://{}", addr)
}

// Receive the next text frame and parse as JSON, skipping non-text frames.
async fn recv_json(
    read: &mut futures::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) -> Value {
    // Give the server up to 2 s to respond
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match read.next().await {
                Some(Ok(Message::Text(t))) => {
                    return serde_json::from_str(&t)
                        .unwrap_or_else(|_| panic!("invalid JSON: {t}"));
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => panic!("WS error: {e}"),
                None => panic!("WS stream ended unexpectedly"),
            }
        }
    })
    .await
    .expect("timed out waiting for server frame")
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ws_sends_welcome_frame_on_connect() {
    let nats = match NatsServer::start().await {
        Some(n) => n,
        None => {
            eprintln!("SKIP: Docker unavailable, skipping ws_sends_welcome_frame_on_connect");
            return;
        }
    };

    let state = AppState::new("us-east".into(), nats.connect().await);
    let ws_url = spawn_ws_server(state).await;

    let (ws, _) = connect_async(format!("{}/ws?user_id=alice", ws_url))
        .await
        .expect("connect");
    let (_, mut read) = ws.split();

    let frame = recv_json(&mut read).await;
    assert_eq!(frame["type"], "connected");
    assert_eq!(frame["user_id"], "alice");
    assert_eq!(frame["region"], "us-east");

    nats.stop().await;
}

#[tokio::test]
async fn ws_auto_generates_user_id_when_not_provided() {
    let nats = match NatsServer::start().await {
        Some(n) => n,
        None => {
            eprintln!("SKIP: Docker unavailable");
            return;
        }
    };

    let state = AppState::new("us-east".into(), nats.connect().await);
    let ws_url = spawn_ws_server(state).await;

    let (ws, _) = connect_async(format!("{}/ws", ws_url))
        .await
        .expect("connect");
    let (_, mut read) = ws.split();

    let frame = recv_json(&mut read).await;
    assert_eq!(frame["type"], "connected");
    let uid = frame["user_id"].as_str().expect("user_id field");
    assert!(
        !uid.is_empty(),
        "auto-generated user_id should not be empty"
    );

    nats.stop().await;
}

#[tokio::test]
async fn local_delivery_same_region() {
    let nats = match NatsServer::start().await {
        Some(n) => n,
        None => {
            eprintln!("SKIP: Docker unavailable");
            return;
        }
    };

    let state = AppState::new("us-east".into(), nats.connect().await);
    let ws_url = spawn_ws_server(state).await;

    // Alice connects
    let (alice_ws, _) = connect_async(format!("{}/ws?user_id=alice", ws_url))
        .await
        .expect("alice connect");
    let (mut alice_write, mut alice_read) = alice_ws.split();
    let _ = recv_json(&mut alice_read).await; // consume welcome

    // Bob connects to the same server
    let (bob_ws, _) = connect_async(format!("{}/ws?user_id=bob", ws_url))
        .await
        .expect("bob connect");
    let (_, mut bob_read) = bob_ws.split();
    let _ = recv_json(&mut bob_read).await; // consume welcome

    // Alice messages Bob
    alice_write
        .send(Message::Text(
            json!({ "to": "bob", "content": "hello local" })
                .to_string()
                .into(),
        ))
        .await
        .expect("alice send");

    // Bob should receive
    let received = recv_json(&mut bob_read).await;
    assert_eq!(received["type"], "message");
    assert_eq!(received["from"], "alice");
    assert_eq!(received["content"], "hello local");
    assert_eq!(received["from_region"], "us-east");

    nats.stop().await;
}

#[tokio::test]
async fn offline_recipient_receives_warning_frame() {
    let nats = match NatsServer::start().await {
        Some(n) => n,
        None => {
            eprintln!("SKIP: Docker unavailable");
            return;
        }
    };

    let state = AppState::new("us-east".into(), nats.connect().await);
    let ws_url = spawn_ws_server(state).await;

    // Only Alice connects — "charlie" is never registered
    let (alice_ws, _) = connect_async(format!("{}/ws?user_id=alice", ws_url))
        .await
        .expect("alice connect");
    let (mut alice_write, mut alice_read) = alice_ws.split();
    let _ = recv_json(&mut alice_read).await; // welcome

    alice_write
        .send(Message::Text(
            json!({ "to": "charlie", "content": "are you there?" })
                .to_string()
                .into(),
        ))
        .await
        .expect("alice send");

    let warning = recv_json(&mut alice_read).await;
    assert_eq!(
        warning["type"], "warning",
        "Expected warning frame, got: {warning}"
    );
    assert!(
        warning["content"]
            .as_str()
            .unwrap_or("")
            .contains("charlie"),
        "Warning should mention the offline user"
    );

    nats.stop().await;
}

#[tokio::test]
async fn presence_is_registered_on_connect() {
    let nats = match NatsServer::start().await {
        Some(n) => n,
        None => {
            eprintln!("SKIP: Docker unavailable");
            return;
        }
    };

    let state = AppState::new("us-east".into(), nats.connect().await);
    let state_clone = state.clone();
    let ws_url = spawn_ws_server(state).await;

    let (ws, _) = connect_async(format!("{}/ws?user_id=probe-user", ws_url))
        .await
        .expect("connect");
    let (_, mut read) = ws.split();
    let _ = recv_json(&mut read).await; // welcome

    tokio::time::sleep(Duration::from_millis(50)).await;

    assert!(
        state_clone.connections.contains_key("probe-user"),
        "probe-user should be in connections map"
    );

    nats.stop().await;
}

#[tokio::test]
async fn presence_is_cleaned_up_on_disconnect() {
    let nats = match NatsServer::start().await {
        Some(n) => n,
        None => {
            eprintln!("SKIP: Docker unavailable");
            return;
        }
    };

    let state = AppState::new("us-east".into(), nats.connect().await);
    let state_clone = state.clone();
    let ws_url = spawn_ws_server(state).await;

    let (ws, _) = connect_async(format!("{}/ws?user_id=temp-user", ws_url))
        .await
        .expect("connect");
    let (mut write, mut read) = ws.split();
    let _ = recv_json(&mut read).await; // welcome

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(state_clone.connections.contains_key("temp-user"));

    write.send(Message::Close(None)).await.expect("close");
    drop(read);

    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(
        !state_clone.connections.contains_key("temp-user"),
        "temp-user should be removed from connections after disconnect"
    );

    nats.stop().await;
}

#[tokio::test]
async fn malformed_json_does_not_crash_server() {
    let nats = match NatsServer::start().await {
        Some(n) => n,
        None => {
            eprintln!("SKIP: Docker unavailable");
            return;
        }
    };

    let state = AppState::new("us-east".into(), nats.connect().await);
    let ws_url = spawn_ws_server(state).await;

    let (alice_ws, _) = connect_async(format!("{}/ws?user_id=alice", ws_url))
        .await
        .expect("connect");
    let (mut alice_write, mut alice_read) = alice_ws.split();
    let _ = recv_json(&mut alice_read).await; // welcome

    // Send garbage — server should swallow it silently
    alice_write
        .send(Message::Text("{{invalid json{{".into()))
        .await
        .expect("send garbage");

    // Connect Bob and confirm server is still alive
    let (bob_ws, _) = connect_async(format!("{}/ws?user_id=bob", ws_url))
        .await
        .expect("bob connect");
    let (_, mut bob_read) = bob_ws.split();
    let _ = recv_json(&mut bob_read).await; // welcome

    alice_write
        .send(Message::Text(
            json!({ "to": "bob", "content": "still alive?" })
                .to_string()
                .into(),
        ))
        .await
        .expect("send valid after garbage");

    let received = recv_json(&mut bob_read).await;
    assert_eq!(received["content"], "still alive?");

    nats.stop().await;
}
