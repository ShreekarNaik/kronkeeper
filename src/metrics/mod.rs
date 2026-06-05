use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

pub fn init() -> PrometheusHandle {
    let builder = PrometheusBuilder::new();
    let handle = builder
        .install_recorder()
        .expect("failed to install Prometheus recorder");

    describe_counter!(
        "jobs_scheduled_total",
        "Total number of jobs created via the API"
    );
    describe_counter!(
        "jobs_dispatched_total",
        "Total number of jobs dispatched to the worker pool"
    );
    describe_counter!(
        "jobs_completed_total",
        "Total number of jobs completed successfully"
    );
    describe_counter!(
        "jobs_failed_total",
        "Total number of jobs that reached a final failure state"
    );
    describe_counter!("jobs_expired_total", "Total number of jobs that hit TTL");
    describe_gauge!(
        "worker_queue_depth",
        "Current number of jobs waiting in the worker channel"
    );
    describe_gauge!(
        "worker_active_count",
        "Number of jobs currently executing"
    );
    describe_histogram!(
        "job_execution_duration_seconds",
        "Job execution duration in seconds"
    );
    describe_counter!(
        "webhook_failures_total",
        "Total number of failed webhook delivery attempts"
    );
    describe_gauge!(
        "scheduler_heap_size",
        "Number of jobs in the scheduler in-memory heap"
    );
    describe_gauge!(
        "recurring_jobs_active",
        "Number of active recurring job templates"
    );
    describe_counter!(
        "recurring_instances_created_total",
        "Total number of recurring job instances created"
    );

    handle
}

pub fn record_job_scheduled() {
    counter!("jobs_scheduled_total").increment(1);
}

pub fn record_job_dispatched() {
    counter!("jobs_dispatched_total").increment(1);
}

pub fn record_job_completed() {
    counter!("jobs_completed_total").increment(1);
}

pub fn record_job_failed() {
    counter!("jobs_failed_total").increment(1);
}

pub fn record_job_expired() {
    counter!("jobs_expired_total").increment(1);
}

pub fn set_worker_queue_depth(depth: usize) {
    gauge!("worker_queue_depth").set(depth as f64);
}

pub fn set_worker_active_count(count: usize) {
    gauge!("worker_active_count").set(count as f64);
}

pub fn record_execution_duration(secs: f64) {
    histogram!("job_execution_duration_seconds").record(secs);
}

pub fn record_webhook_failure() {
    counter!("webhook_failures_total").increment(1);
}

pub fn set_scheduler_heap_size(size: usize) {
    gauge!("scheduler_heap_size").set(size as f64);
}

pub fn set_recurring_jobs_active(count: i64) {
    gauge!("recurring_jobs_active").set(count as f64);
}

pub fn record_recurring_instance_created() {
    counter!("recurring_instances_created_total").increment(1);
}
