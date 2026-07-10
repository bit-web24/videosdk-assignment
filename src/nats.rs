use axum::extract::ws::Message as WsMessage;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use crate::state::AppState;

// Messages sent between regions travel on "messages.<region_id>".
// Each region server only subscribes to its own subject.
// Presence events travel on "presence" — every region subscribes to this
// so their local presence maps stay in sync with the rest of the cluster.

/// Payload forwarded from one region to another when the recipient is remote.
#[derive(Debug, Serialize, Deserialize)]
pub struct RegionMessage {
    pub from: String,
    pub from_region: String,
    pub to: String,
    pub content: String,
}

/// Broadcast whenever a user connects or disconnects from any region.
/// All regions listen to this and update their local presence map.
#[derive(Debug, Serialize, Deserialize)]
pub struct PresenceEvent {
    pub user_id: String,
    pub region_id: String,
    pub kind: PresenceEventKind,
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PresenceEventKind {
    Connected,
    Disconnected,
}

/// Send a message to a specific region via NATS.
/// The target region's `subscribe_messages` task picks it up and delivers locally.
pub async fn publish_to_region(nats: &async_nats::Client, region: &str, msg: RegionMessage) {
    let subject = format!("messages.{}", region);
    match serde_json::to_vec(&msg) {
        Ok(payload) => {
            if let Err(e) = nats.publish(subject, payload.into()).await {
                error!("Failed to publish message to region {}: {}", region, e);
            }
        }
        Err(e) => error!("Failed to serialize RegionMessage: {}", e),
    }
}

/// Broadcast a presence event so all regions update their presence map.
pub async fn publish_presence(nats: &async_nats::Client, event: PresenceEvent) {
    match serde_json::to_vec(&event) {
        Ok(payload) => {
            if let Err(e) = nats.publish("presence", payload.into()).await {
                error!("Failed to publish presence event: {}", e);
            }
        }
        Err(e) => error!("Failed to serialize PresenceEvent: {}", e),
    }
}

/// Subscribes to "messages.<region_id>" and delivers each incoming message
/// to the recipient's WebSocket channel if they are connected locally.
pub async fn subscribe_messages(state: AppState) {
    let subject = format!("messages.{}", state.region_id);

    let mut sub = match state.nats.subscribe(subject.clone()).await {
        Ok(s) => s,
        Err(e) => {
            error!("Could not subscribe to {}: {}", subject, e);
            return;
        }
    };

    info!(
        "Listening for cross-region messages on subject: {}",
        subject
    );

    while let Some(msg) = sub.next().await {
        let envelope = match serde_json::from_slice::<RegionMessage>(&msg.payload) {
            Ok(m) => m,
            Err(e) => {
                error!("Could not deserialize incoming RegionMessage: {}", e);
                continue;
            }
        };

        info!(
            "Received cross-region message from client '{}' (region '{}') for local client '{}'",
            envelope.from, envelope.from_region, envelope.to
        );

        match state.connections.get(&envelope.to) {
            Some(sender) => {
                let payload = serde_json::json!({
                    "type": "message",
                    "from": envelope.from,
                    "from_region": envelope.from_region,
                    "content": envelope.content,
                });
                if sender
                    .send(WsMessage::Text(payload.to_string().into()))
                    .is_err()
                {
                    warn!(
                        "Could not deliver message to '{}' — channel already closed",
                        envelope.to
                    );
                } else {
                    info!(
                        "Delivered cross-region message from client '{}' (region '{}') to local client '{}' (region '{}')",
                        envelope.from, envelope.from_region, envelope.to, state.region_id
                    );
                }
            }
            None => {
                warn!(
                    "User {} not connected locally, dropping message from {}",
                    envelope.to, envelope.from
                );
            }
        }
    }
}

/// Subscribes to "presence" and keeps the local presence map in sync
/// with connect/disconnect events from all other regions.
pub async fn subscribe_presence(state: AppState) {
    let mut sub = match state.nats.subscribe("presence").await {
        Ok(s) => s,
        Err(e) => {
            error!("Could not subscribe to presence subject: {}", e);
            return;
        }
    };

    info!("Listening for presence events on subject: presence");

    while let Some(msg) = sub.next().await {
        let event = match serde_json::from_slice::<PresenceEvent>(&msg.payload) {
            Ok(e) => e,
            Err(e) => {
                error!("Could not deserialize PresenceEvent: {}", e);
                continue;
            }
        };

        match event.kind {
            PresenceEventKind::Connected => {
                info!(
                    "User {} is now present in region {}",
                    event.user_id, event.region_id
                );
                state.presence.insert(event.user_id, event.region_id);
            }
            PresenceEventKind::Disconnected => {
                info!("User {} left region {}", event.user_id, event.region_id);
                state.presence.remove(&event.user_id);
            }
        }
    }
}
