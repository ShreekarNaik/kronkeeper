#!/usr/bin/env bash
set -euo pipefail

BASE_URL="${BASE_URL:-http://localhost:2401}"
API_KEY="${API_KEY:-dev-api-key-change-in-production}"

echo "== Health =="
curl -sf "$BASE_URL/health" | jq .

echo "== Create one-off job =="
JOB_RESP=$(curl -sf -X POST "$BASE_URL/api/v1/jobs" \
  -H "X-API-Key: $API_KEY" \
  -H "Content-Type: application/json" \
  -d '{
    "idempotency_key": "test-one-off-'"$(date +%s)"'",
    "payload": {
      "type": "http",
      "method": "GET",
      "url": "https://httpbin.org/get",
      "timeout_sec": 10
    },
    "scheduled_at": "'"$(date -u -v+5S +%Y-%m-%dT%H:%M:%SZ 2>/dev/null || date -u -d '+5 seconds' +%Y-%m-%dT%H:%M:%SZ)"'"
  }')
echo "$JOB_RESP" | jq .
JOB_ID=$(echo "$JOB_RESP" | jq -r .id)

echo "== Get job =="
curl -sf "$BASE_URL/api/v1/jobs/$JOB_ID" -H "X-API-Key: $API_KEY" | jq .

echo "== Metrics (sample) =="
curl -sf "$BASE_URL/metrics" -H "X-API-Key: $API_KEY" | head -20

echo "All API smoke tests passed."
