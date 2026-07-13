// Unit tests for NATS wire types in src/nats.rs:
//   - RegionMessage round-trip JSON serialization
//   - PresenceEvent round-trip JSON serialization
//   - PresenceEventKind serde snake_case naming
//
// Run with: cargo test --test nats_types_tests

use videosdk_assignment::nats::{PresenceEvent, PresenceEventKind, RegionMessage};

// RegionMessage serialization

#[test]
fn region_message_serializes_to_json() {
    let msg = RegionMessage {
        from: "alice".into(),
        from_region: "us-east".into(),
        to: "bob".into(),
        content: "hello".into(),
    };
    let json = serde_json::to_string(&msg).expect("serialization should succeed");
    assert!(json.contains("\"from\":\"alice\""));
    assert!(json.contains("\"from_region\":\"us-east\""));
    assert!(json.contains("\"to\":\"bob\""));
    assert!(json.contains("\"content\":\"hello\""));
}

#[test]
fn region_message_deserializes_from_json() {
    let raw = r#"{"from":"alice","from_region":"us-east","to":"bob","content":"hello world"}"#;
    let msg: RegionMessage = serde_json::from_str(raw).expect("deserialization should succeed");
    assert_eq!(msg.from, "alice");
    assert_eq!(msg.from_region, "us-east");
    assert_eq!(msg.to, "bob");
    assert_eq!(msg.content, "hello world");
}

#[test]
fn region_message_round_trips_through_json() {
    let original = RegionMessage {
        from: "carol".into(),
        from_region: "ap-south".into(),
        to: "dave".into(),
        content: "round trip test".into(),
    };
    let json = serde_json::to_vec(&original).expect("serialize");
    let restored: RegionMessage = serde_json::from_slice(&json).expect("deserialize");
    assert_eq!(restored.from, original.from);
    assert_eq!(restored.from_region, original.from_region);
    assert_eq!(restored.to, original.to);
    assert_eq!(restored.content, original.content);
}

// PresenceEvent serialization

#[test]
fn presence_event_connected_serializes_correctly() {
    let event = PresenceEvent {
        user_id: "alice".into(),
        region_id: "us-east".into(),
        kind: PresenceEventKind::Connected,
    };
    let json = serde_json::to_string(&event).expect("serialize");
    // serde(rename_all = "snake_case") means Connected -> "connected"
    assert!(json.contains("\"kind\":\"connected\""), "got: {json}");
    assert!(json.contains("\"user_id\":\"alice\""));
    assert!(json.contains("\"region_id\":\"us-east\""));
}

#[test]
fn presence_event_disconnected_serializes_correctly() {
    let event = PresenceEvent {
        user_id: "bob".into(),
        region_id: "eu-west".into(),
        kind: PresenceEventKind::Disconnected,
    };
    let json = serde_json::to_string(&event).expect("serialize");
    assert!(json.contains("\"kind\":\"disconnected\""), "got: {json}");
}

#[test]
fn presence_event_round_trips_connected() {
    let original = PresenceEvent {
        user_id: "user1".into(),
        region_id: "us-east".into(),
        kind: PresenceEventKind::Connected,
    };
    let bytes = serde_json::to_vec(&original).expect("serialize");
    let restored: PresenceEvent = serde_json::from_slice(&bytes).expect("deserialize");
    assert_eq!(restored.user_id, original.user_id);
    assert_eq!(restored.region_id, original.region_id);
    assert_eq!(restored.kind, PresenceEventKind::Connected);
}

#[test]
fn presence_event_round_trips_disconnected() {
    let original = PresenceEvent {
        user_id: "user2".into(),
        region_id: "eu-west".into(),
        kind: PresenceEventKind::Disconnected,
    };
    let bytes = serde_json::to_vec(&original).expect("serialize");
    let restored: PresenceEvent = serde_json::from_slice(&bytes).expect("deserialize");
    assert_eq!(restored.kind, PresenceEventKind::Disconnected);
}

#[test]
fn presence_event_deserializes_from_connected_json() {
    let raw = r#"{"user_id":"alice","region_id":"us-east","kind":"connected"}"#;
    let event: PresenceEvent = serde_json::from_str(raw).expect("deserialize");
    assert_eq!(event.kind, PresenceEventKind::Connected);
}

#[test]
fn presence_event_deserializes_from_disconnected_json() {
    let raw = r#"{"user_id":"bob","region_id":"eu-west","kind":"disconnected"}"#;
    let event: PresenceEvent = serde_json::from_str(raw).expect("deserialize");
    assert_eq!(event.kind, PresenceEventKind::Disconnected);
}

#[test]
fn presence_event_fails_on_unknown_kind() {
    let raw = r#"{"user_id":"alice","region_id":"us-east","kind":"unknown_event"}"#;
    let result = serde_json::from_str::<PresenceEvent>(raw);
    assert!(
        result.is_err(),
        "Unknown kind variant should fail deserialization"
    );
}
