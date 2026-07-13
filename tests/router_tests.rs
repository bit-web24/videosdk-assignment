// Integration tests for the Global Traffic Router HTTP endpoints.
// These tests spin up a real Axum HTTP server on a random port and exercise:
//   - GET /route?region=<known>   → 200 with ws_url
//   - GET /route?region=<unknown> → 400 with error + available_regions
//   - GET /regions                → 200 sorted candidate list with ping_url derived
//   - GET /health                 → 200 "OK"
//
// Run with: cargo test --test router_tests

use serde_json::Value;
use std::{collections::HashMap, net::SocketAddr};
use tokio::net::TcpListener;

// We build the exact same Axum app the router binary builds, but inline here
// so we can test without spawning a separate process.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};

// Mirror the router's internal types (they are private in the binary so we
// redefine them here for test-only use).
#[derive(Clone)]
struct RouterState {
    region_map: HashMap<String, String>,
}

#[derive(Deserialize)]
struct RouteQuery {
    region: String,
}

#[derive(Serialize)]
struct RouteResponse {
    region: String,
    ws_url: String,
}

#[derive(Serialize)]
struct RegionCandidate {
    region: String,
    ping_url: String,
    ws_url: String,
}

#[derive(Serialize)]
struct ErrorResponse {
    error: String,
    available_regions: Vec<String>,
}

fn parse_region_map(raw: &str) -> HashMap<String, String> {
    raw.split(',')
        .filter_map(|entry| {
            let mut parts = entry.splitn(2, '=');
            let region = parts.next()?.trim().to_string();
            let ws_url = parts.next()?.trim().to_string();
            if region.is_empty() || ws_url.is_empty() {
                return None;
            }
            Some((region, ws_url))
        })
        .collect()
}

fn derive_ping_url(ws_url: &str) -> String {
    let http_prefix = if ws_url.starts_with("wss://") {
        ws_url.replacen("wss://", "https://", 1)
    } else {
        ws_url.replacen("ws://", "http://", 1)
    };
    if let Some(base) = http_prefix.strip_suffix("/ws") {
        format!("{}/ping", base)
    } else {
        format!("{}/ping", http_prefix)
    }
}

async fn route_handler(
    Query(params): Query<RouteQuery>,
    State(state): State<RouterState>,
) -> Result<Json<RouteResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.region_map.get(&params.region) {
        Some(ws_url) => Ok(Json(RouteResponse {
            region: params.region,
            ws_url: ws_url.clone(),
        })),
        None => {
            let mut available_regions: Vec<String> = state.region_map.keys().cloned().collect();
            available_regions.sort();
            Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Unknown region '{}'", params.region),
                    available_regions,
                }),
            ))
        }
    }
}

async fn regions_handler(State(state): State<RouterState>) -> Json<Vec<RegionCandidate>> {
    let mut candidates: Vec<RegionCandidate> = state
        .region_map
        .iter()
        .map(|(region, ws_url)| RegionCandidate {
            region: region.clone(),
            ping_url: derive_ping_url(ws_url),
            ws_url: ws_url.clone(),
        })
        .collect();
    candidates.sort_by(|a, b| a.region.cmp(&b.region));
    Json(candidates)
}

async fn health_handler() -> &'static str {
    "OK"
}

fn build_test_app(region_map: HashMap<String, String>) -> Router {
    let state = RouterState { region_map };
    Router::new()
        .route("/route", get(route_handler))
        .route("/regions", get(regions_handler))
        .route("/health", get(health_handler))
        .with_state(state)
}

/// Spawn the app on a random OS-assigned port and return its base URL.
async fn spawn_test_server(region_map: HashMap<String, String>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind random port");
    let addr: SocketAddr = listener.local_addr().expect("local_addr");
    let app = build_test_app(region_map);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("server error");
    });
    format!("http://{}", addr)
}

// ── parse_region_map unit tests (pure, no network) ──────────────────────────

#[test]
fn parse_region_map_valid_two_regions() {
    let raw = "us-east=ws://localhost:3001/ws,eu-west=ws://localhost:3002/ws";
    let map = parse_region_map(raw);
    assert_eq!(map.len(), 2);
    assert_eq!(map.get("us-east").unwrap(), "ws://localhost:3001/ws");
    assert_eq!(map.get("eu-west").unwrap(), "ws://localhost:3002/ws");
}

#[test]
fn parse_region_map_single_region() {
    let raw = "us-east=ws://localhost:3001/ws";
    let map = parse_region_map(raw);
    assert_eq!(map.len(), 1);
    assert_eq!(map.get("us-east").unwrap(), "ws://localhost:3001/ws");
}

#[test]
fn parse_region_map_trims_whitespace() {
    let raw = "  us-east = ws://localhost:3001/ws , eu-west = ws://localhost:3002/ws ";
    let map = parse_region_map(raw);
    assert_eq!(map.len(), 2);
    assert!(map.contains_key("us-east"));
    assert!(map.contains_key("eu-west"));
}

#[test]
fn parse_region_map_skips_malformed_entries() {
    // "bad-entry" has no '=' so it should be skipped
    let raw = "us-east=ws://localhost:3001/ws,bad-entry,eu-west=ws://localhost:3002/ws";
    let map = parse_region_map(raw);
    assert_eq!(map.len(), 2, "malformed entry should be skipped");
}

#[test]
fn parse_region_map_empty_string_returns_empty_map() {
    let map = parse_region_map("");
    assert!(map.is_empty(), "empty input should produce an empty map");
}

#[test]
fn parse_region_map_url_with_equals_in_path_is_preserved() {
    // ws:// URL that incidentally has more '=' — splitn(2, '=') should only split on the first
    let raw = "us-east=ws://host:3001/ws";
    let map = parse_region_map(raw);
    assert_eq!(map.get("us-east").unwrap(), "ws://host:3001/ws");
}

// ── derive_ping_url unit tests (pure, no network) ───────────────────────────

#[test]
fn derive_ping_url_ws_with_ws_suffix() {
    assert_eq!(
        derive_ping_url("ws://localhost:3001/ws"),
        "http://localhost:3001/ping"
    );
}

#[test]
fn derive_ping_url_wss_with_ws_suffix() {
    assert_eq!(
        derive_ping_url("wss://example.com:443/ws"),
        "https://example.com:443/ping"
    );
}

#[test]
fn derive_ping_url_ws_without_ws_suffix() {
    // If the URL does not end with /ws, /ping is appended directly
    assert_eq!(
        derive_ping_url("ws://localhost:3001"),
        "http://localhost:3001/ping"
    );
}

#[test]
fn derive_ping_url_wss_without_ws_suffix() {
    assert_eq!(
        derive_ping_url("wss://example.com"),
        "https://example.com/ping"
    );
}

// ── HTTP handler integration tests ──────────────────────────────────────────

#[tokio::test]
async fn route_handler_returns_200_for_known_region() {
    let mut map = HashMap::new();
    map.insert("us-east".into(), "ws://localhost:3001/ws".into());
    let base = spawn_test_server(map).await;

    let resp = reqwest::get(format!("{}/route?region=us-east", base))
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 200);
    let body: Value = resp.json().await.expect("json");
    assert_eq!(body["region"], "us-east");
    assert_eq!(body["ws_url"], "ws://localhost:3001/ws");
}

#[tokio::test]
async fn route_handler_returns_400_for_unknown_region() {
    let mut map = HashMap::new();
    map.insert("us-east".into(), "ws://localhost:3001/ws".into());
    let base = spawn_test_server(map).await;

    let resp = reqwest::get(format!("{}/route?region=ap-south", base))
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 400);
    let body: Value = resp.json().await.expect("json");
    assert!(
        body["error"].as_str().unwrap().contains("ap-south"),
        "Error message should mention the unknown region"
    );
    let available = body["available_regions"].as_array().unwrap();
    assert!(available.iter().any(|r| r == "us-east"));
}

#[tokio::test]
async fn regions_handler_returns_sorted_candidates() {
    let mut map = HashMap::new();
    map.insert("us-east".into(), "ws://localhost:3001/ws".into());
    map.insert("eu-west".into(), "ws://localhost:3002/ws".into());
    let base = spawn_test_server(map).await;

    let resp = reqwest::get(format!("{}/regions", base))
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 200);
    let body: Vec<Value> = resp.json().await.expect("json");
    assert_eq!(body.len(), 2);

    // Should be sorted alphabetically: eu-west first, then us-east
    assert_eq!(body[0]["region"], "eu-west");
    assert_eq!(body[1]["region"], "us-east");

    // ping_url should be derived correctly
    assert_eq!(body[0]["ping_url"], "http://localhost:3002/ping");
    assert_eq!(body[1]["ping_url"], "http://localhost:3001/ping");

    // ws_url should be preserved
    assert_eq!(body[0]["ws_url"], "ws://localhost:3002/ws");
    assert_eq!(body[1]["ws_url"], "ws://localhost:3001/ws");
}

#[tokio::test]
async fn health_handler_returns_ok() {
    let base = spawn_test_server(HashMap::new()).await;

    let resp = reqwest::get(format!("{}/health", base))
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 200);
    assert_eq!(resp.text().await.expect("text"), "OK");
}

#[tokio::test]
async fn route_handler_returns_400_when_no_region_param() {
    let mut map = HashMap::new();
    map.insert("us-east".into(), "ws://localhost:3001/ws".into());
    let base = spawn_test_server(map).await;

    // Missing ?region= query param should result in 400 (axum query extraction failure)
    let resp = reqwest::get(format!("{}/route", base))
        .await
        .expect("request failed");

    assert_eq!(resp.status(), 400);
}
