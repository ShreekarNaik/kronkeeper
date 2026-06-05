ALTER TABLE jobs ADD COLUMN is_recurring BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE jobs ADD COLUMN cron_expr VARCHAR(100);
ALTER TABLE jobs ADD COLUMN next_instance_at TIMESTAMPTZ;
ALTER TABLE jobs ADD COLUMN max_occurrences INT;
ALTER TABLE jobs ADD COLUMN occurrences_completed INT NOT NULL DEFAULT 0;
ALTER TABLE jobs ADD COLUMN recurrence_end_at TIMESTAMPTZ;
ALTER TABLE jobs ADD COLUMN parent_job_id UUID REFERENCES jobs(id);
ALTER TABLE jobs ADD COLUMN concurrency_policy VARCHAR(20) NOT NULL DEFAULT 'queue_once'
    CHECK (concurrency_policy IN ('skip', 'allow', 'queue_once'));
ALTER TABLE jobs ADD COLUMN is_template BOOLEAN NOT NULL DEFAULT FALSE;

CREATE INDEX idx_jobs_parent_job_id ON jobs (parent_job_id);
CREATE INDEX idx_jobs_is_template ON jobs (is_template) WHERE is_template = TRUE;
