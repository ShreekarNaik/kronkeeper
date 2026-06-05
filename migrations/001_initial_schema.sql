CREATE EXTENSION IF NOT EXISTS "pgcrypto";

CREATE TABLE clients (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name VARCHAR(255) NOT NULL,
    api_key VARCHAR(64) UNIQUE NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE jobs (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    idempotency_key VARCHAR(255) UNIQUE NOT NULL,
    client_id UUID REFERENCES clients(id),
    payload JSONB NOT NULL,
    state VARCHAR(20) NOT NULL DEFAULT 'SCHEDULED'
        CHECK (state IN ('SCHEDULED','LEASED','RUNNING',
                         'COMPLETED','FAILED','EXPIRED',
                         'CANCELLED','DEAD_LETTER')),

    scheduled_at TIMESTAMPTZ NOT NULL,
    leased_until TIMESTAMPTZ,
    expires_at TIMESTAMPTZ,

    attempt_count INT NOT NULL DEFAULT 0,
    max_retries INT NOT NULL DEFAULT 3,
    retry_delay_sec INT NOT NULL DEFAULT 60,

    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    webhook_url TEXT,
    webhook_success BOOLEAN,
    webhook_attempts INT DEFAULT 0
);

CREATE INDEX idx_jobs_state_scheduled ON jobs (scheduled_at) WHERE state = 'SCHEDULED';
CREATE INDEX idx_jobs_leased_until ON jobs (leased_until) WHERE state = 'LEASED';
CREATE INDEX idx_jobs_client_id ON jobs (client_id);
CREATE INDEX idx_jobs_state ON jobs (state);
