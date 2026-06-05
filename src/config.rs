use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub api_listen_addr: SocketAddr,
    pub worker_count: usize,
    pub worker_queue_size: usize,
    pub heap_lookahead_limit: i64,
    pub lease_duration: Duration,
    pub lease_reaper_interval: Duration,
    pub max_retry_backoff_secs: i64,
    pub webhook_timeout: Duration,
    pub webhook_max_retries: i32,
    pub script_safe_dir: PathBuf,
    pub shutdown_timeout: Duration,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url =
            env::var("DATABASE_URL").context("DATABASE_URL must be set")?;

        let api_listen_addr: SocketAddr = env::var("API_LISTEN_ADDR")
            .unwrap_or_else(|_| "0.0.0.0:8080".to_string())
            .parse()
            .context("invalid API_LISTEN_ADDR")?;

        Ok(Self {
            database_url,
            api_listen_addr,
            worker_count: parse_usize("WORKER_COUNT", 10)?,
            worker_queue_size: parse_usize("WORKER_QUEUE_SIZE", 1000)?,
            heap_lookahead_limit: parse_i64("HEAP_LOOKAHEAD_LIMIT", 10_000)?,
            lease_duration: Duration::from_secs(parse_u64("LEASE_DURATION_SECS", 30)?),
            lease_reaper_interval: Duration::from_secs(parse_u64(
                "LEASE_REAPER_INTERVAL_SECS",
                10,
            )?),
            max_retry_backoff_secs: parse_i64("MAX_RETRY_BACKOFF_SECS", 3600)?,
            webhook_timeout: Duration::from_secs(parse_u64("WEBHOOK_TIMEOUT_SECS", 5)?),
            webhook_max_retries: parse_i32("WEBHOOK_MAX_RETRIES", 3)?,
            script_safe_dir: PathBuf::from(
                env::var("SCRIPT_SAFE_DIR").unwrap_or_else(|_| "/opt/daemon/scripts".to_string()),
            ),
            shutdown_timeout: Duration::from_secs(parse_u64("SHUTDOWN_TIMEOUT_SECS", 30)?),
        })
    }
}

fn parse_usize(key: &str, default: usize) -> Result<usize> {
    match env::var(key) {
        Ok(v) => v.parse().with_context(|| format!("invalid {key}")),
        Err(_) => Ok(default),
    }
}

fn parse_i64(key: &str, default: i64) -> Result<i64> {
    match env::var(key) {
        Ok(v) => v.parse().with_context(|| format!("invalid {key}")),
        Err(_) => Ok(default),
    }
}

fn parse_i32(key: &str, default: i32) -> Result<i32> {
    match env::var(key) {
        Ok(v) => v.parse().with_context(|| format!("invalid {key}")),
        Err(_) => Ok(default),
    }
}

fn parse_u64(key: &str, default: u64) -> Result<u64> {
    match env::var(key) {
        Ok(v) => v.parse().with_context(|| format!("invalid {key}")),
        Err(_) => Ok(default),
    }
}
