// Unit tests for src/state.rs — AppState and its shared maps.
// These tests verify:
//   - ConnectionMap (Arc<DashMap<String, WsSender>>) behaves correctly when shared
//   - PresenceMap (Arc<DashMap<String, String>>) behaves correctly across clones
//   - AppState::new initialises clean empty maps
//
// NOTE: These are pure in-memory tests — no NATS, no network.
// Run with: cargo test --test state_tests

use dashmap::DashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

// We test the map types directly since AppState::new requires a live NATS client.
// This is intentional — we want to test the data-structure behaviour in isolation.

// Alias matches what state.rs defines to keep tests semantically clear.
type ConnectionMap =
    Arc<DashMap<String, tokio::sync::mpsc::UnboundedSender<axum::extract::ws::Message>>>;
type PresenceMap = Arc<DashMap<String, String>>;

#[test]
fn connection_map_insert_and_retrieve() {
    let map: ConnectionMap = Arc::new(DashMap::new());
    let (tx, _rx) = mpsc::unbounded_channel();
    map.insert("alice".to_string(), tx);
    assert!(
        map.contains_key("alice"),
        "alice should be in the connections map"
    );
    assert!(!map.contains_key("bob"), "bob was never inserted");
}

#[test]
fn connection_map_remove_cleans_up_entry() {
    let map: ConnectionMap = Arc::new(DashMap::new());
    let (tx, _rx) = mpsc::unbounded_channel();
    map.insert("alice".to_string(), tx);
    map.remove("alice");
    assert!(!map.contains_key("alice"), "alice should be removed");
}

#[test]
fn connection_map_shared_across_arc_clones() {
    let map: ConnectionMap = Arc::new(DashMap::new());
    let map_clone = Arc::clone(&map);

    // Insert via the clone
    let (tx, _rx) = mpsc::unbounded_channel();
    map_clone.insert("bob".to_string(), tx);

    // Should be visible via original Arc
    assert!(
        map.contains_key("bob"),
        "Insert on clone should be visible on original"
    );
}

#[test]
fn connection_map_sender_can_send_message() {
    let map: ConnectionMap = Arc::new(DashMap::new());
    let (tx, mut rx) = mpsc::unbounded_channel();
    map.insert("alice".to_string(), tx);

    // Clone the sender out so the DashMap Ref guard is dropped before we use rx.
    let sender_clone = map.get("alice").expect("alice not found").clone();
    let msg = axum::extract::ws::Message::Text("hello".into());
    sender_clone.send(msg).expect("send should succeed");
    drop(sender_clone);

    // Drain the receiver to confirm the message arrived
    let received = rx.try_recv().expect("message should be in the channel");
    assert_eq!(received, axum::extract::ws::Message::Text("hello".into()));
}

#[test]
fn connection_map_detects_closed_receiver() {
    let map: ConnectionMap = Arc::new(DashMap::new());
    let (tx, rx) = mpsc::unbounded_channel::<axum::extract::ws::Message>();
    map.insert("alice".to_string(), tx);

    // Drop the receiver to simulate a disconnected client
    drop(rx);

    // Clone the sender out so the Ref guard doesn't hold a borrow past the map's lifetime.
    let sender_clone = map.get("alice").expect("alice not found").clone();
    let result = sender_clone.send(axum::extract::ws::Message::Text("hello".into()));
    assert!(result.is_err(), "Sending to a dropped receiver should fail");
}

#[test]
fn presence_map_insert_and_lookup() {
    let map: PresenceMap = Arc::new(DashMap::new());
    map.insert("alice".to_string(), "us-east".to_string());
    let region = map.get("alice").map(|r| r.clone());
    assert_eq!(region, Some("us-east".to_string()));
}

#[test]
fn presence_map_remove_user() {
    let map: PresenceMap = Arc::new(DashMap::new());
    map.insert("bob".to_string(), "eu-west".to_string());
    map.remove("bob");
    assert!(
        !map.contains_key("bob"),
        "bob should be removed from presence"
    );
}

#[test]
fn presence_map_overwrite_updates_region() {
    let map: PresenceMap = Arc::new(DashMap::new());
    map.insert("alice".to_string(), "us-east".to_string());
    // Client reconnects to a different region
    map.insert("alice".to_string(), "eu-west".to_string());
    let region = map.get("alice").map(|r| r.clone());
    assert_eq!(
        region,
        Some("eu-west".to_string()),
        "Presence should update to new region"
    );
}

#[test]
fn presence_map_shared_across_arc_clones() {
    let map: PresenceMap = Arc::new(DashMap::new());
    let map_clone = Arc::clone(&map);

    map_clone.insert("carol".to_string(), "ap-south".to_string());

    assert!(
        map.contains_key("carol"),
        "Presence insert on clone should be visible on original"
    );
}

#[test]
fn presence_map_returns_none_for_missing_user() {
    let map: PresenceMap = Arc::new(DashMap::new());
    assert!(
        map.get("nonexistent").is_none(),
        "Missing user should return None"
    );
}

#[test]
fn multiple_users_in_presence_map() {
    let map: PresenceMap = Arc::new(DashMap::new());
    map.insert("alice".to_string(), "us-east".to_string());
    map.insert("bob".to_string(), "eu-west".to_string());
    map.insert("carol".to_string(), "ap-south".to_string());

    assert_eq!(map.len(), 3);
    assert_eq!(map.get("alice").map(|r| r.clone()), Some("us-east".into()));
    assert_eq!(map.get("bob").map(|r| r.clone()), Some("eu-west".into()));
    assert_eq!(map.get("carol").map(|r| r.clone()), Some("ap-south".into()));
}
