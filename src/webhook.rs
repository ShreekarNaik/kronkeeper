use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tracing::{error, info, warn};

use crate::config::Config;
use crate::db::JobRepository;
use crate::metrics::record_webhook_failure;
use crate::models::JobResponse;

pub struct WebhookEmitter {
    repo: JobRepository,
    client: reqwest::Client,
    config: Arc<Config>,
}

impl WebhookEmitter {
    pub fn new(repo: JobRepository, config: Arc<Config>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(config.webhook_timeout)
            .build()
            .expect("failed to build webhook client");

        Self {
            repo,
            client,
            config,
        }
    }

    pub async fn emit(&self, job_id: uuid::Uuid) {
        let job = match self.repo.get_job_by_id_internal(job_id).await {
            Ok(Some(j)) => j,
            Ok(None) => return,
            Err(e) => {
                error!(job_id = %job_id, error = %e, "failed to load job for webhook");
                return;
            }
        };

        if !job.state.is_terminal() {
            return;
        }

        let Some(url) = job.webhook_url.clone() else {
            return;
        };

        let body = json!({
            "job": JobResponse::from(job.clone()),
            "event": job.state.as_str(),
        });

        let mut attempts = job.webhook_attempts;
        let max_retries = self.config.webhook_max_retries;
        let mut success = false;

        while attempts <= max_retries {
            attempts += 1;
            match self.client.post(&url).json(&body).send().await {
                Ok(resp) if resp.status().is_success() => {
                    success = true;
                    info!(job_id = %job_id, attempts, "webhook delivered");
                    break;
                }
                Ok(resp) => {
                    warn!(
                        job_id = %job_id,
                        status = %resp.status(),
                        attempts,
                        "webhook delivery failed"
                    );
                    record_webhook_failure();
                }
                Err(e) => {
                    warn!(job_id = %job_id, error = %e, attempts, "webhook request error");
                    record_webhook_failure();
                }
            }

            if attempts <= max_retries {
                let backoff = Duration::from_secs(2u64.saturating_pow(attempts as u32));
                tokio::time::sleep(backoff).await;
            }
        }

        if let Err(e) = self
            .repo
            .update_webhook_result(job_id, success, attempts)
            .await
        {
            error!(job_id = %job_id, error = %e, "failed to update webhook result");
        }
    }
}
