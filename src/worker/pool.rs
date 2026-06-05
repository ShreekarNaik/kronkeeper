use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{mpsc, Mutex, Notify};
use tracing::{error, info};

use crate::config::Config;
use crate::db::JobRepository;
use crate::metrics::{self, set_worker_active_count, set_worker_queue_depth};
use crate::models::Job;
use crate::recurring::RecurringService;
use crate::webhook::WebhookEmitter;
use crate::worker::executor::Executor;
use crate::worker::result::ResultHandler;

pub struct WorkerPoolHandle {
    pub tx: mpsc::Sender<Job>,
    shutdown: Arc<Notify>,
    active: Arc<AtomicUsize>,
}

impl WorkerPoolHandle {
    pub fn set_queue_depth(&self, depth: usize) {
        set_worker_queue_depth(depth);
    }

    pub async fn shutdown(&self) {
        self.shutdown.notify_waiters();
    }

    pub async fn wait_for_drain(&self, timeout: std::time::Duration) {
        let start = std::time::Instant::now();
        while self.active.load(Ordering::Relaxed) > 0 {
            if start.elapsed() >= timeout {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    }
}

pub fn spawn_worker_pool(
    config: Arc<Config>,
    repo: JobRepository,
    webhook: Arc<WebhookEmitter>,
    recurring: Arc<RecurringService>,
    scheduler_notify: Arc<Notify>,
) -> WorkerPoolHandle {
    let (tx, rx) = mpsc::channel::<Job>(config.worker_queue_size);
    let rx = Arc::new(Mutex::new(rx));
    let shutdown = Arc::new(Notify::new());
    let active = Arc::new(AtomicUsize::new(0));

    let result_handler = Arc::new(ResultHandler::new(
        JobRepository::new(repo.pool().clone()),
        config.clone(),
        webhook,
        recurring,
        scheduler_notify,
    ));

    for worker_id in 0..config.worker_count {
        let rx = rx.clone();
        let repo = JobRepository::new(repo.pool().clone());
        let config = config.clone();
        let result_handler = result_handler.clone();
        let shutdown = shutdown.clone();
        let active = active.clone();

        tokio::spawn(async move {
            let executor = Executor::new(config.script_safe_dir.clone());
            info!(worker_id, "worker started");

            loop {
                let job = {
                    let recv_fut = async {
                        let mut guard = rx.lock().await;
                        guard.recv().await
                    };

                    tokio::select! {
                        job = recv_fut => job,
                        _ = shutdown.notified() => None,
                    }
                };

                let Some(mut job) = job else {
                    break;
                };

                active.fetch_add(1, Ordering::Relaxed);
                set_worker_active_count(active.load(Ordering::Relaxed));

                if let Err(e) = repo.set_running(job.id).await {
                    error!(job_id = %job.id, error = %e, "failed to set running state");
                    active.fetch_sub(1, Ordering::Relaxed);
                    set_worker_active_count(active.load(Ordering::Relaxed));
                    continue;
                }
                job.state = crate::models::JobState::Running;

                let started = Instant::now();
                let result = executor.execute(&job.payload).await;

                if let Err(e) = result_handler.handle(&job, result, started).await {
                    error!(job_id = %job.id, error = %e, "failed to handle job result");
                }

                active.fetch_sub(1, Ordering::Relaxed);
                set_worker_active_count(active.load(Ordering::Relaxed));
            }

            info!(worker_id, "worker stopped");
        });
    }

    WorkerPoolHandle {
        tx,
        shutdown,
        active,
    }
}
