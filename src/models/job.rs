use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobState {
    Scheduled,
    Leased,
    Running,
    Completed,
    Failed,
    Expired,
    Cancelled,
    DeadLetter,
}

impl JobState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Scheduled => "SCHEDULED",
            Self::Leased => "LEASED",
            Self::Running => "RUNNING",
            Self::Completed => "COMPLETED",
            Self::Failed => "FAILED",
            Self::Expired => "EXPIRED",
            Self::Cancelled => "CANCELLED",
            Self::DeadLetter => "DEAD_LETTER",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "SCHEDULED" => Some(Self::Scheduled),
            "LEASED" => Some(Self::Leased),
            "RUNNING" => Some(Self::Running),
            "COMPLETED" => Some(Self::Completed),
            "FAILED" => Some(Self::Failed),
            "EXPIRED" => Some(Self::Expired),
            "CANCELLED" => Some(Self::Cancelled),
            "DEAD_LETTER" => Some(Self::DeadLetter),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Expired | Self::DeadLetter | Self::Cancelled
        )
    }
}

impl fmt::Display for JobState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum JobPayload {
    Http {
        method: String,
        url: String,
        #[serde(default)]
        headers: HashMap<String, String>,
        #[serde(default)]
        body: Option<String>,
        #[serde(default = "default_timeout")]
        timeout_sec: u64,
    },
    Script {
        path: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default = "default_timeout")]
        timeout_sec: u64,
    },
}

fn default_timeout() -> u64 {
    30
}

impl JobPayload {
    pub fn timeout_sec(&self) -> u64 {
        match self {
            Self::Http { timeout_sec, .. } => *timeout_sec,
            Self::Script { timeout_sec, .. } => *timeout_sec,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ConcurrencyPolicy {
    Skip,
    Allow,
    #[default]
    QueueOnce,
}

impl ConcurrencyPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Allow => "allow",
            Self::QueueOnce => "queue_once",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "skip" => Some(Self::Skip),
            "allow" => Some(Self::Allow),
            "queue_once" => Some(Self::QueueOnce),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecurrenceConfig {
    pub cron_expr: String,
    #[serde(default)]
    pub end_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub max_occurrences: Option<i32>,
    #[serde(default)]
    pub concurrency_policy: ConcurrencyPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateJobRequest {
    pub idempotency_key: String,
    pub payload: JobPayload,
    pub scheduled_at: DateTime<Utc>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default = "default_max_retries")]
    pub max_retries: i32,
    #[serde(default = "default_retry_delay")]
    pub retry_delay_sec: i32,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub recurrence: Option<RecurrenceConfig>,
}

fn default_max_retries() -> i32 {
    3
}

fn default_retry_delay() -> i32 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchJobRequest {
    pub cron_expr: String,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub id: Uuid,
    pub idempotency_key: String,
    pub client_id: Option<Uuid>,
    pub payload: JobPayload,
    pub state: JobState,
    pub scheduled_at: DateTime<Utc>,
    pub leased_until: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub attempt_count: i32,
    pub max_retries: i32,
    pub retry_delay_sec: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub webhook_url: Option<String>,
    pub webhook_success: Option<bool>,
    pub webhook_attempts: i32,
    pub is_recurring: bool,
    pub cron_expr: Option<String>,
    pub next_instance_at: Option<DateTime<Utc>>,
    pub max_occurrences: Option<i32>,
    pub occurrences_completed: i32,
    pub recurrence_end_at: Option<DateTime<Utc>>,
    pub parent_job_id: Option<Uuid>,
    pub concurrency_policy: ConcurrencyPolicy,
    pub is_template: bool,
}

impl Job {
    pub fn from_row(row: JobRow) -> anyhow::Result<Self> {
        let state = JobState::from_str(&row.state)
            .ok_or_else(|| anyhow::anyhow!("unknown job state: {}", row.state))?;
        let payload: JobPayload = serde_json::from_value(row.payload)?;
        let concurrency_policy = ConcurrencyPolicy::from_str(&row.concurrency_policy)
            .unwrap_or(ConcurrencyPolicy::QueueOnce);

        Ok(Self {
            id: row.id,
            idempotency_key: row.idempotency_key,
            client_id: row.client_id,
            payload,
            state,
            scheduled_at: row.scheduled_at,
            leased_until: row.leased_until,
            expires_at: row.expires_at,
            attempt_count: row.attempt_count,
            max_retries: row.max_retries,
            retry_delay_sec: row.retry_delay_sec,
            created_at: row.created_at,
            updated_at: row.updated_at,
            webhook_url: row.webhook_url,
            webhook_success: row.webhook_success,
            webhook_attempts: row.webhook_attempts,
            is_recurring: row.is_recurring,
            cron_expr: row.cron_expr,
            next_instance_at: row.next_instance_at,
            max_occurrences: row.max_occurrences,
            occurrences_completed: row.occurrences_completed,
            recurrence_end_at: row.recurrence_end_at,
            parent_job_id: row.parent_job_id,
            concurrency_policy,
            is_template: row.is_template,
        })
    }
}

#[derive(Debug, sqlx::FromRow)]
pub struct JobRow {
    pub id: Uuid,
    pub idempotency_key: String,
    pub client_id: Option<Uuid>,
    pub payload: serde_json::Value,
    pub state: String,
    pub scheduled_at: DateTime<Utc>,
    pub leased_until: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub attempt_count: i32,
    pub max_retries: i32,
    pub retry_delay_sec: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub webhook_url: Option<String>,
    pub webhook_success: Option<bool>,
    pub webhook_attempts: i32,
    pub is_recurring: bool,
    pub cron_expr: Option<String>,
    pub next_instance_at: Option<DateTime<Utc>>,
    pub max_occurrences: Option<i32>,
    pub occurrences_completed: i32,
    pub recurrence_end_at: Option<DateTime<Utc>>,
    pub parent_job_id: Option<Uuid>,
    pub concurrency_policy: String,
    pub is_template: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct JobResponse {
    pub id: Uuid,
    pub idempotency_key: String,
    pub state: JobState,
    pub payload: JobPayload,
    pub scheduled_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub attempt_count: i32,
    pub max_retries: i32,
    pub retry_delay_sec: i32,
    pub webhook_url: Option<String>,
    pub is_recurring: bool,
    pub cron_expr: Option<String>,
    pub next_instance_at: Option<DateTime<Utc>>,
    pub max_occurrences: Option<i32>,
    pub occurrences_completed: i32,
    pub recurrence_end_at: Option<DateTime<Utc>>,
    pub parent_job_id: Option<Uuid>,
    pub concurrency_policy: ConcurrencyPolicy,
    pub is_template: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<Job> for JobResponse {
    fn from(job: Job) -> Self {
        Self {
            id: job.id,
            idempotency_key: job.idempotency_key,
            state: job.state,
            payload: job.payload,
            scheduled_at: job.scheduled_at,
            expires_at: job.expires_at,
            attempt_count: job.attempt_count,
            max_retries: job.max_retries,
            retry_delay_sec: job.retry_delay_sec,
            webhook_url: job.webhook_url,
            is_recurring: job.is_recurring,
            cron_expr: job.cron_expr,
            next_instance_at: job.next_instance_at,
            max_occurrences: job.max_occurrences,
            occurrences_completed: job.occurrences_completed,
            recurrence_end_at: job.recurrence_end_at,
            parent_job_id: job.parent_job_id,
            concurrency_policy: job.concurrency_policy,
            is_template: job.is_template,
            created_at: job.created_at,
            updated_at: job.updated_at,
        }
    }
}

/// Wrapper for min-heap ordering by scheduled_at (earliest first).
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ScheduledJob {
    pub id: Uuid,
    pub scheduled_at: DateTime<Utc>,
}

impl Ord for ScheduledJob {
    fn cmp(&self, other: &Self) -> Ordering {
        other.scheduled_at.cmp(&self.scheduled_at)
    }
}

impl PartialOrd for ScheduledJob {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn build_heap(jobs: Vec<ScheduledJob>) -> BinaryHeap<ScheduledJob> {
    jobs.into_iter().collect()
}

pub fn compute_retry_delay(retry_delay_sec: i32, attempt_count: i32, max_backoff_secs: i64) -> i64 {
    let exp = attempt_count.saturating_sub(1).max(0) as u32;
    let delay = retry_delay_sec as i64 * 2i64.saturating_pow(exp);
    delay.min(max_backoff_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heap_orders_earliest_first() {
        let now = Utc::now();
        let jobs = vec![
            ScheduledJob {
                id: Uuid::new_v4(),
                scheduled_at: now + chrono::Duration::minutes(5),
            },
            ScheduledJob {
                id: Uuid::new_v4(),
                scheduled_at: now + chrono::Duration::minutes(1),
            },
            ScheduledJob {
                id: Uuid::new_v4(),
                scheduled_at: now + chrono::Duration::minutes(10),
            },
        ];
        let mut heap = build_heap(jobs);
        let first = heap.pop().unwrap();
        let second = heap.pop().unwrap();
        assert!(first.scheduled_at < second.scheduled_at);
    }

    #[test]
    fn backoff_caps_at_max() {
        assert_eq!(compute_retry_delay(60, 1, 3600), 60);
        assert_eq!(compute_retry_delay(60, 2, 3600), 120);
        assert_eq!(compute_retry_delay(60, 10, 3600), 3600);
    }
}
