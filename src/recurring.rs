use std::sync::Arc;

use chrono::{DateTime, Utc};
use cron::Schedule;
use std::str::FromStr;
use tokio::sync::Notify;
use tracing::{info, warn};

use crate::db::JobRepository;
use crate::metrics::{record_recurring_instance_created, set_recurring_jobs_active};
use crate::models::{ConcurrencyPolicy, CreateJobRequest, Job, RecurrenceConfig};

pub struct RecurringService {
    repo: JobRepository,
    notify: Arc<Notify>,
}

impl RecurringService {
    pub fn new(repo: JobRepository, notify: Arc<Notify>) -> Self {
        Self { repo, notify }
    }

    pub async fn refresh_metrics(&self) {
        if let Ok(count) = self.repo.count_recurring_active().await {
            set_recurring_jobs_active(count);
        }
    }

    fn normalize_cron_expr(expr: &str) -> String {
        let parts: Vec<&str> = expr.split_whitespace().collect();
        match parts.len() {
            5 => format!("0 {} *", expr),
            6 => format!("{expr} *"),
            _ => expr.to_string(),
        }
    }

    pub fn next_fire_after(
        cron_expr: &str,
        after: DateTime<Utc>,
    ) -> anyhow::Result<DateTime<Utc>> {
        let normalized = Self::normalize_cron_expr(cron_expr);
        let schedule = Schedule::from_str(&normalized)?;
        schedule
            .after(&after)
            .next()
            .ok_or_else(|| anyhow::anyhow!("no next fire time for cron expression"))
    }

    pub async fn create_recurring(
        &self,
        client_id: uuid::Uuid,
        req: &CreateJobRequest,
        recurrence: &RecurrenceConfig,
    ) -> anyhow::Result<(Job, Job)> {
        let first_fire = Self::next_fire_after(&recurrence.cron_expr, req.scheduled_at)?;

        let template = self
            .repo
            .insert_job(
                client_id,
                req,
                true,
                true,
                Some(&recurrence.cron_expr),
                Some(first_fire),
                recurrence.max_occurrences,
                recurrence.end_at,
                recurrence.concurrency_policy,
                None,
                req.scheduled_at,
            )
            .await?;

        let instance_key = format!("{}:{}", template.id, first_fire.to_rfc3339());
        let instance = self
            .repo
            .insert_instance(&template, first_fire, instance_key)
            .await?;

        record_recurring_instance_created();
        self.refresh_metrics().await;
        self.notify.notify_one();

        Ok((template, instance))
    }

    pub async fn spawn_next_instance(
        &self,
        template_id: uuid::Uuid,
        last_scheduled_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        let template = match self.repo.get_job_by_id_internal(template_id).await? {
            Some(t) if t.is_template && t.is_recurring => t,
            _ => return Ok(()),
        };

        if template.state == crate::models::JobState::Cancelled {
            return Ok(());
        }

        let cron_expr = match &template.cron_expr {
            Some(expr) => expr.clone(),
            None => return Ok(()),
        };

        let next_occurrences = template.occurrences_completed + 1;
        if let Some(max) = template.max_occurrences {
            if next_occurrences >= max {
                self.repo.complete_template(template_id).await?;
                self.refresh_metrics().await;
                info!(template_id = %template_id, "recurring template completed (max occurrences)");
                return Ok(());
            }
        }

        let next_fire = Self::next_fire_after(&cron_expr, last_scheduled_at)?;

        if let Some(end_at) = template.recurrence_end_at {
            if next_fire > end_at {
                self.repo.complete_template(template_id).await?;
                self.refresh_metrics().await;
                info!(template_id = %template_id, "recurring template completed (end date)");
                return Ok(());
            }
        }

        let active = self
            .repo
            .count_active_siblings(template_id, template.concurrency_policy)
            .await?;

        if active > 0 && template.concurrency_policy != ConcurrencyPolicy::Allow {
            warn!(
                template_id = %template_id,
                policy = template.concurrency_policy.as_str(),
                "skipping next instance due to concurrency policy"
            );
            self.repo
                .increment_template_occurrences(template_id, Some(next_fire))
                .await?;
            return Ok(());
        }

        let instance_key = format!("{}:{}", template_id, next_fire.to_rfc3339());
        if self.repo.get_by_idempotency_key(&instance_key).await?.is_some() {
            return Ok(());
        }

        self.repo
            .insert_instance(&template, next_fire, instance_key)
            .await?;
        self.repo
            .increment_template_occurrences(template_id, Some(next_fire))
            .await?;

        record_recurring_instance_created();
        self.refresh_metrics().await;
        self.notify.notify_one();
        info!(template_id = %template_id, next_fire = %next_fire, "spawned recurring instance");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn computes_next_fire_from_cron() {
        let start = Utc.with_ymd_and_hms(2024, 6, 5, 7, 0, 0).unwrap();
        let next = RecurringService::next_fire_after("0 8 * * *", start).unwrap();
        assert_eq!(next.format("%H").to_string(), "08");
    }
}
