use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("Missing required env var: '{0}'")]
    MissingVar(String),

    #[error("'{0}' is not a valid port number")]
    InvalidPort(String),
}

/// Everything the server needs to boot, read from environment variables.
/// Use `Config::from_env()` at startup — fail fast if anything is missing.
#[derive(Debug, Clone)]
pub struct Config {
    /// Identifies this server's region, e.g. "us-east" or "eu-west".
    /// Used as part of NATS subjects and in presence tracking.
    pub region_id: String,

    /// Port to bind the HTTP/WebSocket server on inside the container.
    pub port: u16,

    /// NATS server URL, e.g. "nats://nats:4222".
    pub nats_url: String,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        // Load .env if present. Ignored in production where real env vars are set.
        dotenvy::dotenv().ok();

        let region_id = std::env::var("REGION_ID")
            .map_err(|_| ConfigError::MissingVar("REGION_ID".into()))?;

        let port_str = std::env::var("PORT").unwrap_or_else(|_| "3000".into());
        let port = port_str
            .parse::<u16>()
            .map_err(|_| ConfigError::InvalidPort(port_str))?;

        let nats_url =
            std::env::var("NATS_URL").unwrap_or_else(|_| "nats://localhost:4222".into());

        Ok(Config {
            region_id,
            port,
            nats_url,
        })
    }
}
