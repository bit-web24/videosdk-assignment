use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::{
    nats::{publish_presence, publish_to_region, PresenceEvent, PresenceEventKind, RegionMessage},
    state::AppState,
};

/// Query params accepted on the /ws endpoint.
/// Example: ws://localhost:3001/ws?user_id=alice
#[derive(Deserialize)]
pub struct ConnectQuery {
    pub user_id: Option<String>,
}

/// JSON shape that a connected client sends to route a message.
/// Example: { "to": "bob", "content": "hello" }
#[derive(Deserialize)]
struct OutgoingMessage {
    to: String,
    content: String,
}

/// Axum handler that upgrades an HTTP request to a WebSocket connection.
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(params): Query<ConnectQuery>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    // Use provided user_id or generate one so the client knows who they are.
    let user_id = params
        .user_id
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    ws.on_upgrade(move |socket| handle_socket(socket, user_id, state))
}

async fn handle_socket(socket: WebSocket, user_id: String, state: AppState) {
    let (mut sink, mut stream) = socket.split();

    // Each connection gets an mpsc channel. Any task (NATS subscriber, local router)
    // can push messages into this channel to reach the client.
    let (tx, mut rx) = mpsc::unbounded_channel::<Message>();

    // Register this connection so other parts of the system can find and message this user.
    state.connections.insert(user_id.clone(), tx.clone());

    // Tell all regions this user is now here.
    publish_presence(
        &state.nats,
        PresenceEvent {
            user_id: user_id.clone(),
            region_id: state.region_id.clone(),
            kind: PresenceEventKind::Connected,
        },
    )
    .await;

    info!("User {} connected to region {}", user_id, state.region_id);

    // Send a welcome frame so the client knows their assigned user_id and region.
    let welcome = serde_json::json!({
        "type": "connected",
        "user_id": user_id,
        "region": state.region_id,
    });
    let _ = tx.send(Message::Text(welcome.to_string().into()));

    // Task: drain the outbox channel and write each message to the WebSocket sink.
    let send_task = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink.send(msg).await.is_err() {
                // Client disconnected on the write side.
                break;
            }
        }
    });

    // Task: read frames from the client and route them to the right destination.
    let state_recv = state.clone();
    let uid_recv = user_id.clone();

    let recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = stream.next().await {
            match msg {
                Message::Text(text) => {
                    handle_incoming(&state_recv, &uid_recv, &text).await;
                }
                Message::Close(_) | Message::Binary(_) => break,
                // axum handles Ping/Pong automatically, nothing to do here.
                _ => {}
            }
        }
    });

    // If either direction closes, the connection is done.
    tokio::select! {
        _ = send_task => {}
        _ = recv_task => {}
    }

    // Clean up so stale entries don't affect routing.
    state.connections.remove(&user_id);
    state.presence.remove(&user_id);

    publish_presence(
        &state.nats,
        PresenceEvent {
            user_id: user_id.clone(),
            region_id: state.region_id.clone(),
            kind: PresenceEventKind::Disconnected,
        },
    )
    .await;

    info!(
        "User {} disconnected from region {}",
        user_id, state.region_id
    );
}

/// Decide how to deliver a message from `from_user`:
///   1. Recipient connected locally → push directly into their channel (zero hops).
///   2. Recipient in another region → forward via NATS (one hop).
///   3. Recipient unknown → log and drop.
async fn handle_incoming(state: &AppState, from_user: &str, raw: &str) {
    let msg = match serde_json::from_str::<OutgoingMessage>(raw) {
        Ok(m) => m,
        Err(_) => {
            warn!("Ignoring malformed message from {}: {}", from_user, raw);
            return;
        }
    };

    info!(
        "Received message from client '{}' in region '{}' addressed to client '{}'",
        from_user, state.region_id, msg.to
    );

    // Local delivery — fastest path, no network hop.
    if let Some(sender) = state.connections.get(&msg.to) {
        let payload = serde_json::json!({
            "type": "message",
            "from": from_user,
            "from_region": state.region_id,
            "content": msg.content,
        });
        if sender
            .send(Message::Text(payload.to_string().into()))
            .is_ok()
        {
            info!(
                "Delivered message locally: client '{}' -> client '{}' (region '{}')",
                from_user, msg.to, state.region_id
            );
        }
        return;
    }

    // Cross-region delivery — look up which region holds the recipient.
    if let Some(target_region) = state.presence.get(&msg.to) {
        let envelope = RegionMessage {
            from: from_user.to_string(),
            from_region: state.region_id.clone(),
            to: msg.to.clone(),
            content: msg.content,
        };
        info!(
            "Routing cross-region message: client '{}' (region '{}') -> client '{}' (region '{}') via NATS",
            from_user, state.region_id, msg.to, *target_region
        );
        publish_to_region(&state.nats, &target_region, envelope).await;
        return;
    }

    warn!(
        "Could not route message from '{}' to '{}': target user not found in any region",
        from_user, msg.to
    );

    if let Some(sender) = state.connections.get(from_user) {
        let warning_payload = serde_json::json!({
            "type": "warning",
            "content": format!("User '{}' is currently offline or not present in any region. Message dropped.", msg.to),
        });
        let _ = sender.send(Message::Text(warning_payload.to_string().into()));
    }
}
