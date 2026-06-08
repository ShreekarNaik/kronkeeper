# kronkeeper — deploy bundle

Run kronkeeper with Docker Compose. No Rust toolchain or source code required.

## Prerequisites

- [Docker](https://docs.docker.com/get-docker/)
- Docker Compose v2 (`docker compose`)

## Quick start

```bash
# Unpack the release archive, then:
./up.sh
```

`up.sh` loads the bundled Docker image (if present), creates `.env` from `.env.example` on first run, and starts PostgreSQL + kronkeeper.

The API is available at `http://localhost:2401` (or the port set in `API_PORT`).

## Verify

```bash
curl http://localhost:2401/health
```

## Configuration

Copy and edit environment variables:

```bash
cp .env.example .env
```

Important production changes:

1. Set strong `POSTGRES_PASSWORD` values.
2. Replace the seeded dev API key — migration `003_seed_dev_client.sql` inserts `dev-api-key-change-in-production`. Add your own client in Postgres and rotate before going live:

```sql
INSERT INTO clients (name, api_key) VALUES ('my-service', 'your-secret-api-key');
```

Restart kronkeeper after adding clients: `docker compose restart kronkeeper`.

## Scripts

Place executable job scripts in `./scripts/`. They are mounted read-only at `/opt/daemon/scripts` inside the container.

## Operations

```bash
docker compose ps          # status
docker compose logs -f     # follow logs
docker compose down        # stop (keeps database volume)
docker compose down -v     # stop and delete database volume
```

## Using a registry image instead of the tarball

If you publish the image to a registry, set `KRONKEEPER_IMAGE` in `.env` and remove or ignore `kronkeeper-image.tar`:

```bash
KRONKEEPER_IMAGE=ghcr.io/your-org/kronkeeper:0.1.0
docker compose pull
docker compose up -d
```
