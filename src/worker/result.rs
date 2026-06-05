use std::sync::Arc;
use std::time::Instant;

use chrono::{Duration as ChronoDuration, Utc};
use tokio::sync::Notify;
use tracing::{error, info};

use crate::config::Config;
use crate::db::JobRepository;
use crate::metrics::{self, record_execution_duration};
use crate::models::{compute_retry_delay, Job, JobState};
use crate::recurring::RecurringService;
use crate::webhook::WebhookEmitter;
use crate::worker::executor::ExecutionResult;

pub struct ResultHandler {
    repo: JobRepository,
    config: Arc<Config>,
    webhook: Arc<WebhookEmitter>,
    recurring: Arc<RecurringService>,
    notify: Arc<Notify>,
}

impl ResultHandler {
    pub fn new(
        repo: JobRepository,
        config: Arc<Config>,
        webhook: Arc<WebhookEmitter>,
        recurring: Arc<RecurringService>,
        notify: Arc<Notify>,
    ) -> Self {
        Self {
            repo,
            config,
            webhook,
            recurring,
            notify,
        }
    }

    pub async fn handle(
        &self,
        job: &Job,
        result: ExecutionResult,
        started_at: Instant,
    ) -> anyhow::Result<()> {
        let duration = started_at.elapsed().as_secs_f64();
        record_execution_duration(duration);

        let now = Utc::now();

        if result.success {
            self.repo.set_completed(job.id).await?;
            metrics::record_job_completed();
            info!(job_id = %job.id, "job completed");

            if let Some(parent_id) = job.parent_job_id {
                self.recurring
                    .spawn_next_instance(parent_id, job.scheduled_at)
                    .await?;
            }

            self.webhook.emit(job.id).await;
            return Ok(());
        }

        if let Some(expires_at) = job.expires_at {
            if now >= expires_at {
                self.repo.set_state(job.id, JobState::Expired).await?;
                metrics::record_job_expired();
                info!(job_id = %job.id, "job expired");
                self.webhook.emit(job.id).await;
                return Ok(());
            }
        }

        let next_attempt = job.attempt_count + 1;
        if next_attempt > job.max_retries {
            self.repo.set_state(job.id, JobState::DeadLetter).await?;
            metrics::record_job_failed();
            error!(job_id = %job.id, output = %result.output, "job dead lettered");
            self.webhook.emit(job.id).await;
            return Ok(());
        }

        let delay_secs = compute_retry_delay(
            job.retry_delay_sec,
            next_attempt,
            self.config.max_retry_backoff_secs,
        );
        let retry_at = now + ChronoDuration::seconds(delay_secs);
        self.repo
            .schedule_retry(job.id, retry_at, next_attempt)
            .await?;
        info!(
            job_id = %job.id,
            attempt = next_attempt,
            retry_at = %retry_at,
            "job scheduled for retry"
        );
        self.notify.notify_one();
        Ok(())
    }
}
