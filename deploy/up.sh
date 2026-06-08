#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

if [[ -f kronkeeper-image.tar ]]; then
  echo "Loading kronkeeper Docker image..."
  docker load -i kronkeeper-image.tar
fi

if [[ ! -f .env ]]; then
  cp .env.example .env
  echo "Created .env from .env.example — review credentials before production use."
fi

docker compose up -d

API_PORT="$(grep -E '^API_PORT=' .env 2>/dev/null | cut -d= -f2- || echo 2401)"
API_PORT="${API_PORT:-2401}"

echo ""
echo "kronkeeper is starting."
echo "  API:    http://localhost:${API_PORT}"
echo "  Health: curl http://localhost:${API_PORT}/health"
echo ""
echo "Stop with: docker compose down"
