mod api;
mod config;
mod db;
mod metrics;
mod models;
mod reaper;
mod recurring;
mod scheduler;
mod webhook;
mod worker;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::Notify;
use tracing::info;
use tracing_subscriber::EnvFilter;

use crate::api::handlers::jobs::AppState;
use crate::api::middleware::auth::{load_api_keys, AuthState};
use crate::config::Config;
use crate::db::{create_pool, JobRepository};
use crate::recurring::RecurringService;
use crate::scheduler::Scheduler;
use crate::webhook::WebhookEmitter;
use crate::worker::spawn_worker_pool;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let config = Arc::new(Config::from_env()?);
    let pool = create_pool(&config.database_url).await?;
    let repo = JobRepository::new(pool.clone());

    let recovered = reaper::recover_on_startup(&repo).await?;
    if recovered > 0 {
        info!(recovered, "startup recovery complete");
    }

    let notify = Arc::new(Notify::new());
    let scheduler_alive = Arc::new(AtomicBool::new(false));

    let webhook = Arc::new(WebhookEmitter::new(
        JobRepository::new(pool.clone()),
        config.clone(),
    ));

    let recurring = Arc::new(RecurringService::new(
        JobRepository::new(pool.clone()),
        notify.clone(),
    ));
    recurring.refresh_metrics().await;

    let worker_pool = spawn_worker_pool(
        config.clone(),
        JobRepository::new(pool.clone()),
        webhook,
        recurring.clone(),
        notify.clone(),
    );

    let scheduler = Scheduler::new(
        JobRepository::new(pool.clone()),
        notify.clone(),
        worker_pool.tx.clone(),
        config.clone(),
        scheduler_alive.clone(),
    );
    tokio::spawn(scheduler.run());

    let reaper_repo = JobRepository::new(pool.clone());
    let reaper_notify = notify.clone();
    let reaper_interval = config.lease_reaper_interval;
    tokio::spawn(async move {
        reaper::run_lease_reaper(reaper_repo, reaper_notify, reaper_interval).await;
    });

    let api_keys = load_api_keys(&repo).await?;
    let auth_state = AuthState { api_keys };

    let metrics_handle = metrics::init();

    let app_state = AppState {
        repo: JobRepository::new(pool.clone()),
        notify: notify.clone(),
        recurring,
    };

    let router = api::build_router(api::routes::ServerState {
        pool: pool.clone(),
        app: app_state,
        auth: auth_state,
        metrics: metrics_handle,
        scheduler_alive,
    });

    let listener = tokio::net::TcpListener::bind(config.api_listen_addr).await?;
    info!(addr = %config.api_listen_addr, "kronkeeper listening");

    let shutdown_timeout = config.shutdown_timeout;
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("shutting down worker pool");
    worker_pool.shutdown().await;
    worker_pool
        .wait_for_drain(shutdown_timeout)
        .await;

    info!("kronkeeper stopped");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received");
}
