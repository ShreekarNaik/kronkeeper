# kronkeeper

A production-oriented job scheduler and executor written in Rust. Submit jobs over a REST API, persist them in PostgreSQL, run them through a bounded worker pool, and receive completion callbacks via webhooks.

## Features

- **Durable scheduling** — PostgreSQL is the source of truth; jobs survive process restarts.
- **Event-driven scheduler** — sleeps until the next deadline instead of polling.
- **Bounded concurrency** — fixed-size worker pool and queue with backpressure.
- **Fault tolerance** — leases, automatic retries with exponential backoff, TTL expiry, and idempotency keys.
- **Recurring jobs** — cron-based templates that spawn instances on schedule.
- **Two execution modes** — outbound HTTP requests or local scripts (from a safe directory).
- **Observability** — structured JSON logs, Prometheus metrics, and a health endpoint.

## Quick start (Docker)

Prerequisites: [Docker](https://docs.docker.com/get-docker/) and Docker Compose.

```bash
# Start PostgreSQL + kronkeeper
docker compose up --build -d

# Wait for the API to come up, then smoke-test
./scripts/test_api.sh
```

The API listens on `http://localhost:8080`. A development API key is seeded by migration `003_seed_dev_client.sql`:

| Field   | Value                              |
|---------|------------------------------------|
| API key | `dev-api-key-change-in-production` |

Change this before deploying to production.

## Local development

### Prerequisites

- Rust 1.85+ (see `Dockerfile`)
- PostgreSQL 16+

### 1. Start PostgreSQL

```bash
docker compose up -d postgres
```

Or point `DATABASE_URL` at an existing Postgres instance.

### 2. Configure environment

```bash
cp .env.example .env
# Edit .env if needed — DATABASE_URL is required
```

### 3. Run the daemon

```bash
cargo run --release
```

Migrations in `migrations/` run automatically on startup.

### 4. (Optional) Run API smoke tests

```bash
./scripts/test_api.sh
```

## Configuration

All settings are loaded from environment variables. See [`.env.example`](.env.example) for the full list.

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | *(required)* | PostgreSQL connection string |
| `API_LISTEN_ADDR` | `0.0.0.0:8080` | HTTP bind address |
| `WORKER_COUNT` | `10` | Number of concurrent workers |
| `WORKER_QUEUE_SIZE` | `1000` | Max jobs buffered in the worker queue |
| `HEAP_LOOKAHEAD_LIMIT` | `10000` | Max scheduled jobs loaded into the scheduler heap |
| `LEASE_DURATION_SECS` | `30` | How long a leased job is reserved before reaping |
| `LEASE_REAPER_INTERVAL_SECS` | `10` | Interval for reclaiming expired leases |
| `MAX_RETRY_BACKOFF_SECS` | `3600` | Cap on exponential retry delay |
| `WEBHOOK_TIMEOUT_SECS` | `5` | Per-request webhook timeout |
| `WEBHOOK_MAX_RETRIES` | `3` | Webhook delivery attempts |
| `SCRIPT_SAFE_DIR` | `/opt/daemon/scripts` | Directory allowed for script execution |
| `SHUTDOWN_TIMEOUT_SECS` | `30` | Grace period for in-flight jobs on shutdown |
| `RUST_LOG` | `info` | Log filter (`tracing` / `env-filter` syntax) |

## Authentication

Protected endpoints require an `X-API-Key` header. API keys are stored in the `clients` table and loaded at startup.

To add a client manually:

```sql
INSERT INTO clients (name, api_key)
VALUES ('my-service', 'your-secret-api-key');
```

Restart kronkeeper (or redeploy) so the new key is picked up.

## API reference

Base URL: `http://localhost:8080`

### Public endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Liveness — checks database and scheduler |
| `GET` | `/metrics` | Prometheus metrics |

### Protected endpoints

All require `X-API-Key`.

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/v1/jobs` | Create a one-off or recurring job |
| `GET` | `/api/v1/jobs/{id}` | Get job status |
| `DELETE` | `/api/v1/jobs/{id}` | Cancel a scheduled job (or recurring template) |
| `PATCH` | `/api/v1/jobs/{id}` | Update the cron expression on a recurring template |

### Create a one-off HTTP job

```bash
curl -X POST http://localhost:8080/api/v1/jobs \
  -H "X-API-Key: dev-api-key-change-in-production" \
  -H "Content-Type: application/json" \
  -d '{
    "idempotency_key": "send-welcome-email-42",
    "payload": {
      "type": "http",
      "method": "POST",
      "url": "https://api.example.com/notify",
      "headers": { "Content-Type": "application/json" },
      "body": "{\"user_id\": 42}",
      "timeout_sec": 30
    },
    "scheduled_at": "2026-06-05T12:00:00Z",
    "expires_at": "2026-06-05T18:00:00Z",
    "max_retries": 3,
    "retry_delay_sec": 60,
    "webhook_url": "https://api.example.com/webhooks/jobs"
  }'
```

Re-submitting the same `idempotency_key` returns the existing job (HTTP 200) instead of creating a duplicate.

### Create a recurring job

```bash
curl -X POST http://localhost:8080/api/v1/jobs \
  -H "X-API-Key: dev-api-key-change-in-production" \
  -H "Content-Type: application/json" \
  -d '{
    "idempotency_key": "nightly-report",
    "payload": {
      "type": "script",
      "path": "hello.sh",
      "args": [],
      "timeout_sec": 120
    },
    "scheduled_at": "2026-06-05T02:00:00Z",
    "recurrence": {
      "cron_expr": "0 2 * * *",
      "max_occurrences": 30,
      "concurrency_policy": "queue_once"
    },
    "webhook_url": "https://api.example.com/webhooks/jobs"
  }'
```

`concurrency_policy` options: `queue_once` (default), `skip`, `allow`.

### Run a local script

Place executable scripts under `SCRIPT_SAFE_DIR`. With Docker Compose, the `scripts/` directory is mounted read-only at `/opt/daemon/scripts`.

```bash
curl -X POST http://localhost:8080/api/v1/jobs \
  -H "X-API-Key: dev-api-key-change-in-production" \
  -H "Content-Type: application/json" \
  -d '{
    "idempotency_key": "run-hello",
    "payload": {
      "type": "script",
      "path": "hello.sh",
      "timeout_sec": 30
    },
    "scheduled_at": "2026-06-05T12:00:00Z"
  }'
```

### Get job status

```bash
curl http://localhost:8080/api/v1/jobs/<job-id> \
  -H "X-API-Key: dev-api-key-change-in-production"
```

### Cancel a job

```bash
curl -X DELETE http://localhost:8080/api/v1/jobs/<job-id> \
  -H "X-API-Key: dev-api-key-change-in-production"
```

Only `SCHEDULED` jobs can be cancelled. Cancelling a recurring template also cancels pending instances.

## Job lifecycle

```
SCHEDULED → LEASED → RUNNING → COMPLETED
                ↓         ↓
            (reaper)   FAILED → SCHEDULED (retry) or DEAD_LETTER / EXPIRED
SCHEDULED → CANCELLED
```

Terminal states: `COMPLETED`, `FAILED`, `EXPIRED`, `CANCELLED`, `DEAD_LETTER`.

## Webhooks

When a `webhook_url` is set, kronkeeper POSTs a JSON payload on terminal state transitions:

```json
{
  "job": { "...": "JobResponse fields" },
  "event": "COMPLETED"
}
```

Delivery is retried up to `WEBHOOK_MAX_RETRIES` times.

## Health check

```bash
curl http://localhost:8080/health
```

Returns `200` when both the database and scheduler are healthy; `503` otherwise.

## Project layout

```
├── src/              # Application source (see CONTRIBUTORS.md for module map)
├── migrations/       # SQLx migrations (run on startup)
├── scripts/          # Example scripts + API smoke tests
├── CONTRIBUTORS.md   # Layout conventions and growth rules
├── docker-compose.yml
├── Dockerfile
└── .env.example
```

## License

Not specified — add a `LICENSE` file if you plan to distribute this project.
