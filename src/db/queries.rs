use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::{
    ConcurrencyPolicy, CreateJobRequest, Job, JobRow, JobState, ScheduledJob,
};

fn decode_err(e: anyhow::Error) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        e.to_string(),
    )))
}

const JOB_COLUMNS: &str = r#"
    id, idempotency_key, client_id, payload, state,
    scheduled_at, leased_until, expires_at,
    attempt_count, max_retries, retry_delay_sec,
    created_at, updated_at,
    webhook_url, webhook_success, webhook_attempts,
    is_recurring, cron_expr, next_instance_at,
    max_occurrences, occurrences_completed, recurrence_end_at,
    parent_job_id, concurrency_policy, is_template
"#;

#[derive(Clone)]
pub struct JobRepository {
    pool: PgPool,
}

impl JobRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn load_api_keys(&self) -> Result<Vec<(String, Uuid)>, sqlx::Error> {
        let rows: Vec<(String, Uuid)> =
            sqlx::query_as("SELECT api_key, id FROM clients").fetch_all(&self.pool).await?;
        Ok(rows)
    }

    pub async fn get_by_idempotency_key(
        &self,
        idempotency_key: &str,
    ) -> Result<Option<Job>, sqlx::Error> {
        let query = format!(
            "SELECT {JOB_COLUMNS} FROM jobs WHERE idempotency_key = $1"
        );
        let row: Option<JobRow> = sqlx::query_as(&query)
            .bind(idempotency_key)
            .fetch_optional(&self.pool)
            .await?;
        row.map(Job::from_row).transpose().map_err(decode_err)
    }

    pub async fn get_job_by_id(
        &self,
        id: Uuid,
        client_id: Uuid,
    ) -> Result<Option<Job>, sqlx::Error> {
        let query = format!(
            "SELECT {JOB_COLUMNS} FROM jobs WHERE id = $1 AND client_id = $2"
        );
        let row: Option<JobRow> = sqlx::query_as(&query)
            .bind(id)
            .bind(client_id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(Job::from_row).transpose().map_err(decode_err)
    }

    pub async fn insert_job(
        &self,
        client_id: Uuid,
        req: &CreateJobRequest,
        is_template: bool,
        is_recurring: bool,
        cron_expr: Option<&str>,
        next_instance_at: Option<DateTime<Utc>>,
        max_occurrences: Option<i32>,
        recurrence_end_at: Option<DateTime<Utc>>,
        concurrency_policy: ConcurrencyPolicy,
        parent_job_id: Option<Uuid>,
        scheduled_at: DateTime<Utc>,
    ) -> Result<Job, sqlx::Error> {
        let payload = serde_json::to_value(&req.payload).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let state = JobState::Scheduled.as_str();
        let query = format!(
            r#"
            INSERT INTO jobs (
                idempotency_key, client_id, payload, state,
                scheduled_at, expires_at, max_retries, retry_delay_sec,
                webhook_url, is_recurring, cron_expr, next_instance_at,
                max_occurrences, recurrence_end_at, parent_job_id,
                concurrency_policy, is_template
            ) VALUES (
                $1, $2, $3, $4,
                $5, $6, $7, $8,
                $9, $10, $11, $12,
                $13, $14, $15,
                $16, $17
            )
            RETURNING {JOB_COLUMNS}
            "#
        );

        let row: JobRow = sqlx::query_as(&query)
            .bind(&req.idempotency_key)
            .bind(client_id)
            .bind(payload)
            .bind(state)
            .bind(scheduled_at)
            .bind(req.expires_at)
            .bind(req.max_retries)
            .bind(req.retry_delay_sec)
            .bind(&req.webhook_url)
            .bind(is_recurring)
            .bind(cron_expr)
            .bind(next_instance_at)
            .bind(max_occurrences)
            .bind(recurrence_end_at)
            .bind(parent_job_id)
            .bind(concurrency_policy.as_str())
            .bind(is_template)
            .fetch_one(&self.pool)
            .await?;

        Job::from_row(row).map_err(decode_err)
    }

    pub async fn insert_instance(
        &self,
        template: &Job,
        scheduled_at: DateTime<Utc>,
        idempotency_key: String,
    ) -> Result<Job, sqlx::Error> {
        let req = CreateJobRequest {
            idempotency_key,
            payload: template.payload.clone(),
            scheduled_at,
            expires_at: template.expires_at,
            max_retries: template.max_retries,
            retry_delay_sec: template.retry_delay_sec,
            webhook_url: template.webhook_url.clone(),
            recurrence: None,
        };

        self.insert_job(
            template.client_id.unwrap_or_default(),
            &req,
            false,
            false,
            None,
            None,
            None,
            None,
            template.concurrency_policy,
            Some(template.id),
            scheduled_at,
        )
        .await
    }

    pub async fn fetch_scheduled_jobs(&self, limit: i64) -> Result<Vec<ScheduledJob>, sqlx::Error> {
        let rows: Vec<(Uuid, DateTime<Utc>)> = sqlx::query_as(
            r#"
            SELECT id, scheduled_at
            FROM jobs
            WHERE state = 'SCHEDULED' AND is_template = FALSE
            ORDER BY scheduled_at ASC
            LIMIT $1
            "#,
        )
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|(id, scheduled_at)| ScheduledJob { id, scheduled_at })
            .collect())
    }

    pub async fn get_job_by_id_internal(&self, id: Uuid) -> Result<Option<Job>, sqlx::Error> {
        let query = format!("SELECT {JOB_COLUMNS} FROM jobs WHERE id = $1");
        let row: Option<JobRow> = sqlx::query_as(&query)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        row.map(Job::from_row).transpose().map_err(decode_err)
    }

    pub async fn lease_job(
        &self,
        id: Uuid,
        lease_duration: Duration,
    ) -> Result<Option<Job>, sqlx::Error> {
        let secs = lease_duration.as_secs() as i64;
        let query = format!(
            r#"
            UPDATE jobs
            SET state = 'LEASED',
                leased_until = NOW() + ($2 * INTERVAL '1 second'),
                updated_at = NOW()
            WHERE id = $1 AND state = 'SCHEDULED'
            RETURNING {JOB_COLUMNS}
            "#
        );
        let row: Option<JobRow> = sqlx::query_as(&query)
            .bind(id)
            .bind(secs)
            .fetch_optional(&self.pool)
            .await?;
        row.map(Job::from_row).transpose().map_err(decode_err)
    }

    pub async fn set_running(&self, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE jobs SET state = 'RUNNING', updated_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_completed(&self, id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE jobs SET state = 'COMPLETED', updated_at = NOW() WHERE id = $1",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn set_state(&self, id: Uuid, state: JobState) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE jobs SET state = $2, updated_at = NOW() WHERE id = $1")
            .bind(id)
            .bind(state.as_str())
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn schedule_retry(
        &self,
        id: Uuid,
        scheduled_at: DateTime<Utc>,
        attempt_count: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET state = 'SCHEDULED',
                scheduled_at = $2,
                attempt_count = $3,
                leased_until = NULL,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(scheduled_at)
        .bind(attempt_count)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn cancel_job(&self, id: Uuid, client_id: Uuid) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE jobs
            SET state = 'CANCELLED', updated_at = NOW()
            WHERE id = $1 AND client_id = $2 AND state = 'SCHEDULED'
            "#,
        )
        .bind(id)
        .bind(client_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn cancel_template_and_instances(
        &self,
        template_id: Uuid,
        client_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let template = sqlx::query(
            r#"
            UPDATE jobs
            SET state = 'CANCELLED', updated_at = NOW()
            WHERE id = $1 AND client_id = $2 AND is_template = TRUE
            "#,
        )
        .bind(template_id)
        .bind(client_id)
        .execute(&self.pool)
        .await?;

        let instances = sqlx::query(
            r#"
            UPDATE jobs
            SET state = 'CANCELLED', updated_at = NOW()
            WHERE parent_job_id = $1 AND state = 'SCHEDULED'
            "#,
        )
        .bind(template_id)
        .execute(&self.pool)
        .await?;

        Ok(template.rows_affected() + instances.rows_affected())
    }

    pub async fn update_cron_expr(
        &self,
        id: Uuid,
        client_id: Uuid,
        cron_expr: &str,
    ) -> Result<bool, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE jobs
            SET cron_expr = $3, updated_at = NOW()
            WHERE id = $1 AND client_id = $2 AND is_template = TRUE AND is_recurring = TRUE
            "#,
        )
        .bind(id)
        .bind(client_id)
        .bind(cron_expr)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn reap_expired_leases(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE jobs
            SET state = 'SCHEDULED', leased_until = NULL, updated_at = NOW()
            WHERE state = 'LEASED' AND leased_until < NOW()
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn recover_on_startup(&self) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            r#"
            UPDATE jobs
            SET state = 'SCHEDULED', leased_until = NULL, updated_at = NOW()
            WHERE state IN ('LEASED', 'RUNNING')
              AND (leased_until IS NULL OR leased_until < NOW())
            "#,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn update_webhook_result(
        &self,
        id: Uuid,
        success: bool,
        attempts: i32,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET webhook_success = $2, webhook_attempts = $3, updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id)
        .bind(success)
        .bind(attempts)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn increment_template_occurrences(
        &self,
        template_id: Uuid,
        next_instance_at: Option<DateTime<Utc>>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            r#"
            UPDATE jobs
            SET occurrences_completed = occurrences_completed + 1,
                next_instance_at = $2,
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(template_id)
        .bind(next_instance_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn complete_template(&self, template_id: Uuid) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE jobs SET state = 'COMPLETED', updated_at = NOW() WHERE id = $1",
        )
        .bind(template_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn count_active_siblings(
        &self,
        parent_id: Uuid,
        policy: ConcurrencyPolicy,
    ) -> Result<i64, sqlx::Error> {
        let states: &[&str] = match policy {
            ConcurrencyPolicy::Skip => &["LEASED", "RUNNING"],
            ConcurrencyPolicy::QueueOnce => &["SCHEDULED", "LEASED", "RUNNING"],
            ConcurrencyPolicy::Allow => return Ok(0),
        };

        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM jobs
            WHERE parent_job_id = $1 AND state = ANY($2)
            "#,
        )
        .bind(parent_id)
        .bind(states)
        .fetch_one(&self.pool)
        .await?;

        Ok(count.0)
    }

    pub async fn count_recurring_active(&self) -> Result<i64, sqlx::Error> {
        let count: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM jobs
            WHERE is_template = TRUE AND is_recurring = TRUE
              AND state NOT IN ('COMPLETED', 'CANCELLED', 'DEAD_LETTER')
            "#,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(count.0)
    }
}
