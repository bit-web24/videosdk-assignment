use axum::extract::ws::Message;
use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

/// Channel handle to deliver a message to a locally connected WebSocket client.
pub type WsSender = UnboundedSender<Message>;

/// Map of locally connected users: user_id -> channel sender.
pub type ConnectionMap = Arc<DashMap<String, WsSender>>;

/// Global presence map: user_id -> region_id across all regions.
pub type PresenceMap = Arc<DashMap<String, String>>;

/// Shared application state passed to HTTP/WebSocket handlers and background tasks.
#[derive(Clone)]
pub struct AppState {
    pub region_id: String,
    pub connections: ConnectionMap,
    pub presence: PresenceMap,
    pub nats: async_nats::Client,
}

impl AppState {
    pub fn new(region_id: String, nats: async_nats::Client) -> Self {
        Self {
            region_id,
            connections: Arc::new(DashMap::new()),
            presence: Arc::new(DashMap::new()),
            nats,
        }
    }
}
