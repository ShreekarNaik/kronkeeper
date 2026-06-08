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

The API listens on `http://localhost:2401` (if that port is busy, kronkeeper tries the next port until one is free). A development API key is seeded by migration `003_seed_dev_client.sql`:

| Field   | Value                              |
| ------- | ---------------------------------- |
| API key | `dev-api-key-change-in-production` |

Change this before deploying to production.

## Local development

### Prerequisites

- Rust 1.88+ (see `Dockerfile`)
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

### 5. End-to-end tests (Docker)

The E2E suite spins up an isolated stack (PostgreSQL, kronkeeper, and a webhook/HTTP target), schedules real jobs ~12 seconds in the future, and verifies execution end-to-end.

**Prerequisites:** Docker, Docker Compose, `curl`, and `jq`.

```bash
./tests/e2e/run.sh
```

The runner builds images, waits for the API to become healthy, runs ten scenario tests, then tears the stack down. Set `KEEP_E2E_STACK=1` to leave containers running for debugging.

| Test                        | What it verifies                                                                                          |
| --------------------------- | --------------------------------------------------------------------------------------------------------- |
| Health endpoint             | Database and scheduler are up                                                                             |
| Prometheus metrics endpoint | Protected `/metrics` (401 without key), Prometheus types, counters/histogram update after a job completes |
| API key authentication      | Missing/invalid keys return 401                                                                           |
| Scheduled script job        | `hello.sh` runs and reaches `COMPLETED`                                                                   |
| Scheduled HTTP job          | Outbound GET to internal echo server succeeds                                                             |
| Webhook delivery            | Completion callback is POSTed to webhook receiver                                                         |
| Idempotency                 | Duplicate `idempotency_key` returns HTTP 200 with same job id                                             |
| Cancel scheduled job        | `DELETE` moves a future job to `CANCELLED`                                                                |
| Failed job dead letter      | `fail.sh` with `max_retries: 0` reaches `DEAD_LETTER`                                                     |
| Recurring job lifecycle     | Create instance + template, patch cron, cancel template                                                   |

Unit tests (retry backoff, cron parsing, script path sandboxing) run via `cargo test` and do not require Docker.

## Configuration

All settings are loaded from environment variables. See [`.env.example`](.env.example) for the full list.

| Variable                     | Default               | Description                                       |
| ---------------------------- | --------------------- | ------------------------------------------------- |
| `DATABASE_URL`               | _(required)_          | PostgreSQL connection string                      |
| `API_LISTEN_ADDR`            | `0.0.0.0:2401`        | HTTP bind address (increments port if unavailable) |
| `WORKER_COUNT`               | `10`                  | Number of concurrent workers                      |
| `WORKER_QUEUE_SIZE`          | `1000`                | Max jobs buffered in the worker queue             |
| `HEAP_LOOKAHEAD_LIMIT`       | `10000`               | Max scheduled jobs loaded into the scheduler heap |
| `LEASE_DURATION_SECS`        | `30`                  | How long a leased job is reserved before reaping  |
| `LEASE_REAPER_INTERVAL_SECS` | `10`                  | Interval for reclaiming expired leases            |
| `MAX_RETRY_BACKOFF_SECS`     | `3600`                | Cap on exponential retry delay                    |
| `WEBHOOK_TIMEOUT_SECS`       | `5`                   | Per-request webhook timeout                       |
| `WEBHOOK_MAX_RETRIES`        | `3`                   | Webhook delivery attempts                         |
| `SCRIPT_SAFE_DIR`            | `/opt/daemon/scripts` | Directory allowed for script execution            |
| `SHUTDOWN_TIMEOUT_SECS`      | `30`                  | Grace period for in-flight jobs on shutdown       |
| `RUST_LOG`                   | `info`                | Log filter (`tracing` / `env-filter` syntax)      |

## Authentication

Protected endpoints require an `X-API-Key` header. API keys are stored in the `clients` table and loaded at startup.

To add a client manually:

```sql
INSERT INTO clients (name, api_key)
VALUES ('my-service', 'your-secret-api-key');
```

Restart kronkeeper (or redeploy) so the new key is picked up.

## API reference

Base URL: `http://localhost:2401`

### Public endpoints

| Method | Path      | Description                              |
| ------ | --------- | ---------------------------------------- |
| `GET`  | `/health` | Liveness — checks database and scheduler |

### Protected endpoints

All require `X-API-Key`.

| Method   | Path                | Description                                        |
| -------- | ------------------- | -------------------------------------------------- |
| `GET`    | `/metrics`          | Prometheus metrics                                 |
| `POST`   | `/api/v1/jobs`      | Create a one-off or recurring job                  |
| `GET`    | `/api/v1/jobs/{id}` | Get job status                                     |
| `DELETE` | `/api/v1/jobs/{id}` | Cancel a scheduled job (or recurring template)     |
| `PATCH`  | `/api/v1/jobs/{id}` | Update the cron expression on a recurring template |

### Create a one-off HTTP job

```bash
curl -X POST http://localhost:2401/api/v1/jobs \
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
curl -X POST http://localhost:2401/api/v1/jobs \
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
curl -X POST http://localhost:2401/api/v1/jobs \
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
curl http://localhost:2401/api/v1/jobs/<job-id> \
  -H "X-API-Key: dev-api-key-change-in-production"
```

### Cancel a job

```bash
curl -X DELETE http://localhost:2401/api/v1/jobs/<job-id> \
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
curl http://localhost:2401/health
```

Returns `200` when both the database and scheduler are healthy; `503` otherwise.

## Metrics

`GET /metrics` requires the same `X-API-Key` header as job endpoints:

```bash
curl http://localhost:2401/metrics \
  -H "X-API-Key: dev-api-key-change-in-production"
```

When configuring Prometheus, set a scrape `authorization` header or use a relabel/proxy that injects `X-API-Key`.

```
├── src/                    # Application source (see CONTRIBUTING.md for module map)
├── migrations/             # SQLx migrations (run on startup)
├── scripts/                # Example scripts, API smoke tests
├── tests/e2e/              # Docker-based end-to-end test suite
├── CONTRIBUTING.md         # Layout conventions and growth rules
├── docker-compose.yml
├── Dockerfile
└── .env.example
```

---

## Documentation

### Architecture overview

kronkeeper is a **single-binary daemon** that accepts jobs over HTTP, persists them in PostgreSQL, and executes them through a bounded Tokio worker pool. The scheduler uses an in-memory min-heap backed by the database — it sleeps until the next deadline and wakes when the schedule changes (new job, cancel, retry, etc.).

```
┌─────────────┐     POST /jobs      ┌──────────────────────────────────────────┐
│   Client    │ ──────────────────► │              kronkeeper                  │
│  (service)  │ ◄── webhook POST ── │  API ─► DB ◄── Scheduler ─► Worker pool  │
└─────────────┘                     │                    │              │       │
                                    │                    └── Reaper       │       │
                                    │                    └── Recurring ◄──┘       │
                                    └──────────────────────────────────────────┘
                                                      │
                                                      ▼
                                               PostgreSQL
```

**Startup sequence**

1. Load config from environment; connect to PostgreSQL and run migrations.
2. Recover orphaned `LEASED` / `RUNNING` jobs from a prior crash (reaper).
3. Spawn the scheduler loop, lease reaper, worker pool, and HTTP server.
4. On shutdown (SIGINT/SIGTERM), stop accepting work, drain in-flight jobs, then exit.

### Components

| Component       | Source             | Responsibility                                                                                         |
| --------------- | ------------------ | ------------------------------------------------------------------------------------------------------ |
| **API**         | `src/api/`         | Axum REST interface — job CRUD, health, metrics. API-key middleware on protected routes.               |
| **Database**    | `src/db/`          | SQLx repository — job persistence, leasing, retries, recurring templates/instances.                    |
| **Models**      | `src/models/`      | Domain types — job states, payloads, cron/recurrence config, API request/response shapes.              |
| **Scheduler**   | `src/scheduler.rs` | Event-driven loop — loads upcoming jobs into a heap, leases due jobs, dispatches to workers.           |
| **Worker pool** | `src/worker/`      | Bounded `mpsc` queue of concurrent executors. Runs HTTP or script payloads with timeouts.              |
| **Recurring**   | `src/recurring.rs` | Cron templates — spawns child instances after each successful run; supports concurrency policies.      |
| **Reaper**      | `src/reaper.rs`    | Reclaims expired leases and expired TTL jobs; runs on startup and on an interval.                      |
| **Webhook**     | `src/webhook.rs`   | Async completion callbacks — POSTs job + event JSON with retries.                                      |
| **Metrics**     | `src/metrics.rs`   | Prometheus counters/gauges — jobs scheduled/completed/failed, worker depth, webhooks, recurring count. |
| **Config**      | `src/config.rs`    | Environment-driven settings (workers, leases, backoff, script directory, etc.).                        |

### Feature reference

| Feature                  | Description                                                                         | API / config                                         |
| ------------------------ | ----------------------------------------------------------------------------------- | ---------------------------------------------------- |
| **One-off jobs**         | Run once at `scheduled_at`.                                                         | `POST /api/v1/jobs` without `recurrence`.            |
| **HTTP execution**       | Outbound request with method, URL, headers, body, timeout.                          | `payload.type = "http"`.                             |
| **Script execution**     | Run a script from `SCRIPT_SAFE_DIR` only (path traversal blocked).                  | `payload.type = "script"`.                           |
| **Scheduling**           | Jobs wait in `SCHEDULED` until deadline; scheduler leases and dispatches.           | `scheduled_at` (RFC 3339 UTC).                       |
| **Leases**               | Prevents double execution after crashes. Expired leases are reaped.                 | `LEASE_DURATION_SECS`, `LEASE_REAPER_INTERVAL_SECS`. |
| **Retries**              | Failed jobs reschedule with exponential backoff capped by `MAX_RETRY_BACKOFF_SECS`. | `max_retries`, `retry_delay_sec`.                    |
| **TTL / expiry**         | Jobs past `expires_at` move to `EXPIRED` instead of retrying.                       | `expires_at` on create.                              |
| **Dead letter**          | Jobs that exhaust retries land in `DEAD_LETTER`.                                    | Automatic when `attempt_count > max_retries`.        |
| **Idempotency**          | Same `idempotency_key` returns the existing job (HTTP 200).                         | Unique per job across the system.                    |
| **Cancellation**         | Only `SCHEDULED` jobs cancel; recurring templates cancel pending instances too.     | `DELETE /api/v1/jobs/{id}`.                          |
| **Recurring jobs**       | Cron template spawns instances; patch cron on template.                             | `recurrence.cron_expr`, `PATCH` on template id.      |
| **Concurrency policies** | `queue_once`, `skip`, or `allow` overlapping instances.                             | `recurrence.concurrency_policy`.                     |
| **Webhooks**             | Terminal-state POST with job snapshot and event name.                               | `webhook_url` on create.                             |
| **Authentication**       | `X-API-Key` header; keys loaded from `clients` table at startup.                    | Migration seed or manual `INSERT`.                   |
| **Health**               | Checks DB ping and scheduler heartbeat.                                             | `GET /health` → 200 or 503.                          |
| **Metrics**              | Prometheus text exposition (requires API key).                                      | `GET /metrics` with `X-API-Key`.                     |
| **Graceful shutdown**    | Waits up to `SHUTDOWN_TIMEOUT_SECS` for in-flight jobs.                             | SIGINT / SIGTERM.                                    |

### Job state machine

```
SCHEDULED ──► LEASED ──► RUNNING ──► COMPLETED
    │             │           │
    │             │           └──► FAILED ──► SCHEDULED (retry)
    │             │                      └──► DEAD_LETTER / EXPIRED
    │             └──► SCHEDULED (lease reaper)
    └──► CANCELLED
```

Terminal states: `COMPLETED`, `FAILED`, `EXPIRED`, `CANCELLED`, `DEAD_LETTER`.

### Data model (PostgreSQL)

- **`clients`** — API keys for authentication.
- **`jobs`** — All job rows including recurring templates (`is_template = true`) and spawned instances (`parent_job_id` points to template). Payload stored as JSONB; state, scheduling, retry, and webhook columns track lifecycle.

Indexes optimize queries for scheduled jobs, leased jobs, and client scoping.

### Testing strategy

| Layer     | Command                 | Scope                                                                                           |
| --------- | ----------------------- | ----------------------------------------------------------------------------------------------- |
| **Unit**  | `cargo test`            | Retry backoff math, scheduler heap ordering, cron next-fire, script sandbox paths.              |
| **Smoke** | `./scripts/test_api.sh` | Quick API check against a running instance (create + get job, health, metrics).                 |
| **E2E**   | `./tests/e2e/run.sh`    | Full Docker stack — real scheduling delay, execution, webhooks, auth, recurring, failure paths. |

E2E stack files live under `tests/e2e/`:

- `docker-compose.e2e.yml` — postgres + kronkeeper + webhook-receiver
- `webhook-receiver/` — lightweight Python server used as HTTP job target and webhook capture
- `lib.sh` / `run.sh` — helpers and test scenarios

For deeper design rationale and schema details, see [`.cursor/blueprint.md`](.cursor/blueprint.md) (internal architecture blueprint).
