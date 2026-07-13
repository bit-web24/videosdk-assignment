// Unit tests for src/config.rs
// Environment variable tests must be serialized to prevent races.
// We use a global Mutex so other test suites can still parallelize freely.
// Run with: cargo test --test config_tests

use std::sync::Mutex;
use videosdk_assignment::config::Config;

// Global lock to serialize env-var mutations across all tests in this file.
static ENV_LOCK: Mutex<()> = Mutex::new(());

// Set env vars for the duration of a closure then restore originals.
fn with_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
    let _guard = ENV_LOCK.lock().unwrap();

    // Save current values
    let saved: Vec<(&str, Option<String>)> = vars
        .iter()
        .map(|(k, _)| (*k, std::env::var(k).ok()))
        .collect();

    // Apply test values
    for (k, v) in vars {
        match v {
            Some(val) => std::env::set_var(k, val),
            None => std::env::remove_var(k),
        }
    }

    f();

    // Restore originals
    for (k, original) in &saved {
        match original {
            Some(val) => std::env::set_var(k, val),
            None => std::env::remove_var(k),
        }
    }
}

#[test]
fn config_loads_all_fields_from_env() {
    with_env(
        &[
            ("REGION_ID", Some("us-east")),
            ("PORT", Some("3001")),
            ("NATS_URL", Some("nats://custom:4222")),
        ],
        || {
            let cfg = Config::from_env().expect("Config should load successfully");
            assert_eq!(cfg.region_id, "us-east");
            assert_eq!(cfg.port, 3001);
            assert_eq!(cfg.nats_url, "nats://custom:4222");
        },
    );
}

#[test]
fn config_port_defaults_to_3000_when_not_set() {
    with_env(
        &[
            ("REGION_ID", Some("eu-west")),
            ("PORT", None),
            ("NATS_URL", None),
        ],
        || {
            let cfg = Config::from_env().expect("Config should load with PORT default");
            assert_eq!(cfg.port, 3000);
        },
    );
}

#[test]
fn config_nats_url_defaults_when_not_set() {
    with_env(
        &[
            ("REGION_ID", Some("ap-south")),
            ("PORT", None),
            ("NATS_URL", None),
        ],
        || {
            let cfg = Config::from_env().expect("Config should load with NATS default");
            assert_eq!(cfg.nats_url, "nats://localhost:4222");
        },
    );
}

#[test]
fn config_fails_when_region_id_is_missing() {
    with_env(
        &[
            ("REGION_ID", None),
            ("PORT", Some("3001")),
            ("NATS_URL", Some("nats://localhost:4222")),
        ],
        || {
            let result = Config::from_env();
            assert!(
                result.is_err(),
                "Config should fail when REGION_ID is absent"
            );
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("REGION_ID"),
                "Error should mention REGION_ID, got: {msg}"
            );
        },
    );
}

#[test]
fn config_fails_when_port_is_not_a_number() {
    with_env(
        &[
            ("REGION_ID", Some("us-east")),
            ("PORT", Some("not-a-port")),
            ("NATS_URL", Some("nats://localhost:4222")),
        ],
        || {
            let result = Config::from_env();
            assert!(result.is_err(), "Config should fail for non-numeric PORT");
            let msg = result.unwrap_err().to_string();
            assert!(
                msg.contains("not-a-port"),
                "Error should mention the invalid value, got: {msg}"
            );
        },
    );
}

#[test]
fn config_fails_when_port_exceeds_u16_range() {
    with_env(
        &[
            ("REGION_ID", Some("us-east")),
            ("PORT", Some("99999")),
            ("NATS_URL", Some("nats://localhost:4222")),
        ],
        || {
            let result = Config::from_env();
            assert!(result.is_err(), "Config should fail for port > 65535");
        },
    );
}
