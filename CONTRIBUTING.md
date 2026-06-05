# Contributing to kronkeeper

## Source layout

The codebase favors **minimal structure with clear subsystem boundaries**. Each top-level module should be describable in one sentence.

```
src/
├── main.rs
├── config.rs
│
├── api/
│   ├── error.rs
│   ├── routes.rs
│   ├── mod.rs
│   ├── handlers/
│   │   ├── health.rs
│   │   ├── jobs.rs
│   │   ├── metrics.rs
│   │   └── mod.rs
│   └── middleware/
│       ├── auth.rs
│       └── mod.rs
│
├── db/
│   ├── mod.rs
│   ├── pool.rs
│   └── queries.rs
│
├── models/
│   ├── client.rs
│   ├── job.rs
│   └── mod.rs
│
├── worker/
│   ├── executor.rs
│   ├── pool.rs
│   ├── result.rs
│   └── mod.rs
│
├── scheduler.rs
├── recurring.rs
├── webhook.rs
├── reaper.rs
└── metrics.rs
```

### Module responsibilities

| Module     | Responsibility                          |
|------------|-----------------------------------------|
| `api`      | HTTP interface                          |
| `db`       | Persistence                             |
| `models`   | Domain types                            |
| `worker`   | Executes jobs                           |
| `scheduler`| When jobs run                           |
| `recurring`| Cron / recurring job logic              |
| `reaper`   | Lease recovery and cleanup              |
| `webhook`  | Completion callbacks                    |
| `metrics`  | Observability (Prometheus instrumentation) |

## Rules for future growth

### 1. Start flat

Prefer a single file:

```
scheduler.rs
```

over a directory:

```
scheduler/
└── mod.rs
```

until the module actually needs multiple files.

### 2. Split by responsibility, not file size alone

Avoid catch-all modules:

```
utils.rs
helpers.rs
common.rs
```

Prefer focused decomposition:

```
scheduler/
├── heap.rs
├── dispatcher.rs
└── wakeup.rs
```

Each file should own a distinct responsibility.

### 3. Only create a directory when one of these becomes true

- The module exceeds roughly 300–500 LOC and contains multiple concepts.
- Multiple engineers are actively modifying it.
- It naturally contains several components.

Example evolution:

```
scheduler.rs  →  scheduler/mod.rs + heap.rs + dispatcher.rs + wakeup.rs
```

### 4. Watch these hotspots

These files are the most likely to need decomposition first:

- `db/queries.rs`
- `models/job.rs`
- `scheduler.rs`

### 5. Avoid premature enterprise layers

Do **not** add layers such as:

```
services/
repositories/
usecases/
ports/
adapters/
```

unless a concrete problem appears. At the current scale (~3k LOC), those layers usually add complexity without paying for themselves.

### 6. Keep top-level modules capability-oriented

A healthy structure passes this test: every top-level module can be described in one sentence (see the table above). If you cannot describe a module that way, reconsider the boundary.

## Development workflow

1. Copy `.env.example` to `.env` and configure `DATABASE_URL`.
2. Start Postgres: `docker compose up -d postgres`
3. Build and test: `cargo build && cargo test`
4. Run locally: `cargo run`

See [README.md](README.md) for API usage, Docker setup, and deployment notes.

## Pull requests

- Keep changes scoped to one subsystem when possible.
- Run `cargo fmt`, `cargo clippy`, and `cargo test` before opening a PR.
- Add or update tests when changing behavior (especially `recurring`, `scheduler`, and `db/queries`).
- Do not introduce new top-level directories without a reason that matches the rules above.
