mod config;
mod nats;
mod state;
mod ws;

use axum::{routing::get, Router};
use std::net::SocketAddr;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    // Initialize structured logging controlled by RUST_LOG env var (defaults to info if unset)
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    // Load server configuration from environment variables / .env file
    let config = match config::Config::from_env() {
        Ok(cfg) => cfg,
        Err(e) => {
            error!("Configuration error: {}", e);
            std::process::exit(1);
        }
    };

    info!(
        "Starting WebSocket Server for region '{}' on port {}",
        config.region_id, config.port
    );

    // Connect to NATS messaging server
    let nats_client = match async_nats::connect(&config.nats_url).await {
        Ok(client) => {
            info!("Connected to NATS at {}", config.nats_url);
            client
        }
        Err(e) => {
            error!("Failed to connect to NATS at {}: {}", config.nats_url, e);
            std::process::exit(1);
        }
    };

    // Initialize shared application state
    let state = state::AppState::new(config.region_id.clone(), nats_client);

    // Spawn background tasks to listen for inter-region messages and presence events
    tokio::spawn(nats::subscribe_messages(state.clone()));
    tokio::spawn(nats::subscribe_presence(state.clone()));

    // Build the Axum router
    let app = Router::new()
        .route("/ws", get(ws::ws_handler))
        .route("/health", get(health_handler))
        .with_state(state);

    // Bind socket and serve requests
    let addr: SocketAddr = format!("0.0.0.0:{}", config.port)
        .parse()
        .expect("Invalid bind address");

    info!("Listening on {}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            error!("Failed to bind TCP listener to {}: {}", addr, e);
            std::process::exit(1);
        }
    };

    if let Err(e) = axum::serve(listener, app).await {
        error!("Server crashed: {}", e);
        std::process::exit(1);
    }
}

/// Simple health check endpoint used by Docker / load balancer probes.
async fn health_handler() -> &'static str {
    "OK"
}
