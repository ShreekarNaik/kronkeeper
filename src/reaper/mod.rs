use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;
use tracing::{error, info};

use crate::db::JobRepository;

pub async fn recover_on_startup(repo: &JobRepository) -> anyhow::Result<u64> {
    let count = repo.recover_on_startup().await?;
    if count > 0 {
        info!(count, "recovered stale jobs on startup");
    }
    Ok(count)
}

pub async fn run_lease_reaper(
    repo: JobRepository,
    notify: Arc<Notify>,
    interval: Duration,
) {
    info!("lease reaper started");
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;
        match repo.reap_expired_leases().await {
            Ok(count) => {
                if count > 0 {
                    info!(count, "reaped expired leases");
                    notify.notify_one();
                }
            }
            Err(e) => error!(error = %e, "lease reaper failed"),
        }
    }
}
