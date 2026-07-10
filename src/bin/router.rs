use axum::{
    extract::{Query, State},
    http::StatusCode,
    routing::get,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, net::SocketAddr};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

// The router reads REGION_MAP from the environment at startup.
// Format: "us-east=ws://localhost:3001/ws,eu-west=ws://localhost:3002/ws"
// Each entry maps a region name to the public WebSocket URL clients should connect to.

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

/// Parses "us-east=ws://host:3001/ws,eu-west=ws://host:3002/ws" into a HashMap.
fn parse_region_map(raw: &str) -> HashMap<String, String> {
    raw.split(',')
        .filter_map(|entry| {
            // Split on first '=' only so the ws:// URL (which contains '//') is preserved.
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

/// Returns the WebSocket URL for the requested region.
/// If the region is unknown, responds with 400 and lists available regions.
async fn route_handler(
    Query(params): Query<RouteQuery>,
    State(state): State<RouterState>,
) -> Result<Json<RouteResponse>, (StatusCode, Json<ErrorResponse>)> {
    match state.region_map.get(&params.region) {
        Some(ws_url) => {
            info!("Routing client to region '{}' -> {}", params.region, ws_url);
            Ok(Json(RouteResponse {
                region: params.region,
                ws_url: ws_url.clone(),
            }))
        }
        None => {
            warn!("Unknown region requested: '{}'", params.region);
            let available_regions = state.region_map.keys().cloned().collect();
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

/// Converts a WebSocket URL like ws://localhost:3001/ws to an HTTP ping endpoint http://localhost:3001/ping.
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

/// Returns all available regional servers and their ping/WebSocket URLs for client latency probing.
async fn regions_handler(State(state): State<RouterState>) -> Json<Vec<RegionCandidate>> {
    let mut candidates = Vec::new();
    for (region, ws_url) in &state.region_map {
        candidates.push(RegionCandidate {
            region: region.clone(),
            ping_url: derive_ping_url(ws_url),
            ws_url: ws_url.clone(),
        });
    }
    // Sort alphabetically by region name for deterministic output
    candidates.sort_by(|a, b| a.region.cmp(&b.region));
    Json(candidates)
}

async fn health_handler() -> &'static str {
    "OK"
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    dotenvy::dotenv().ok();

    let raw_region_map = match std::env::var("REGION_MAP") {
        Ok(val) => val,
        Err(_) => {
            error!("REGION_MAP env var is required. Example: us-east=ws://localhost:3001/ws,eu-west=ws://localhost:3002/ws");
            std::process::exit(1);
        }
    };

    let region_map = parse_region_map(&raw_region_map);
    if region_map.is_empty() {
        error!("REGION_MAP parsed to an empty map — check the format");
        std::process::exit(1);
    }

    info!("Loaded {} region(s):", region_map.len());
    for (region, url) in &region_map {
        info!("  {} -> {}", region, url);
    }

    let port: u16 = std::env::var("ROUTER_PORT")
        .unwrap_or_else(|_| "8080".into())
        .parse()
        .unwrap_or(8080);

    let state = RouterState { region_map };

    let app = Router::new()
        .route("/route", get(route_handler))
        .route("/regions", get(regions_handler))
        .route("/health", get(health_handler))
        .with_state(state);

    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse().unwrap();
    info!("Global Traffic Router listening on {}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        error!("Router crashed: {}", e);
        std::process::exit(1);
    }
}
