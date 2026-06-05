use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use tokio::sync::{mpsc, Notify};
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::JobRepository;
use crate::metrics::{record_job_dispatched, set_scheduler_heap_size};
use crate::models::{build_heap, Job, ScheduledJob};

pub struct Scheduler {
    repo: JobRepository,
    notify: Arc<Notify>,
    worker_tx: mpsc::Sender<Job>,
    config: Arc<Config>,
    heap: BinaryHeap<ScheduledJob>,
    alive: Arc<AtomicBool>,
}

impl Scheduler {
    pub fn new(
        repo: JobRepository,
        notify: Arc<Notify>,
        worker_tx: mpsc::Sender<Job>,
        config: Arc<Config>,
        alive: Arc<AtomicBool>,
    ) -> Self {
        Self {
            repo,
            notify,
            worker_tx,
            config,
            heap: BinaryHeap::new(),
            alive,
        }
    }

    pub async fn run(mut self) {
        info!("scheduler started");
        if let Err(e) = self.refresh_heap().await {
            error!(error = %e, "initial heap refresh failed");
        }

        loop {
            self.alive.store(true, Ordering::Relaxed);

            let delay = self
                .heap
                .peek()
                .map(|j| (j.scheduled_at - Utc::now()).to_std().unwrap_or(Duration::ZERO))
                .unwrap_or(Duration::from_secs(3600));
            let sleep_fut = tokio::time::sleep(delay);

            tokio::select! {
                _ = sleep_fut => {
                    if let Err(e) = self.dispatch_due_jobs().await {
                        error!(error = %e, "dispatch failed");
                    }
                }
                _ = self.notify.notified() => {
                    if let Err(e) = self.refresh_heap().await {
                        error!(error = %e, "heap refresh failed");
                    }
                }
            }
        }
    }

    async fn refresh_heap(&mut self) -> anyhow::Result<()> {
        let jobs = self
            .repo
            .fetch_scheduled_jobs(self.config.heap_lookahead_limit)
            .await?;
        self.heap = build_heap(jobs);
        set_scheduler_heap_size(self.heap.len());
        Ok(())
    }

    async fn dispatch_due_jobs(&mut self) -> anyhow::Result<()> {
        let now = Utc::now();

        while let Some(peek) = self.heap.peek() {
            if peek.scheduled_at > now {
                break;
            }

            let scheduled = self.heap.pop().unwrap();

            let leased = self
                .repo
                .lease_job(scheduled.id, self.config.lease_duration)
                .await?;

            let Some(job) = leased else {
                continue;
            };

            match self.worker_tx.try_send(job) {
                Ok(()) => {
                    record_job_dispatched();
                }
                Err(mpsc::error::TrySendError::Full(job)) => {
                    warn!(job_id = %job.id, "worker queue full, returning job to scheduled");
                    self.repo
                        .schedule_retry(job.id, job.scheduled_at, job.attempt_count)
                        .await?;
                    self.notify.notify_one();
                    break;
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    error!("worker channel closed, scheduler stopping dispatch");
                    break;
                }
            }
        }

        Ok(())
    }
}
