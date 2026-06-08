#!/usr/bin/env bash
# Shared helpers for kronkeeper E2E tests.

set -euo pipefail

E2E_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$E2E_DIR/../.." && pwd)"
COMPOSE_FILE="$E2E_DIR/docker-compose.e2e.yml"

BASE_URL="${BASE_URL:-http://localhost:2401}"
WEBHOOK_RECEIVER_URL="${WEBHOOK_RECEIVER_URL:-http://localhost:9090}"
API_KEY="${API_KEY:-dev-api-key-change-in-production}"

# In-container URLs (used in job payloads).
HTTP_TARGET_BASE="http://webhook-receiver:8080"
WEBHOOK_URL="${HTTP_TARGET_BASE}/webhook"

TESTS_RUN=0
TESTS_PASSED=0

require_tools() {
  for tool in curl jq docker; do
    if ! command -v "$tool" >/dev/null 2>&1; then
      echo "Missing required tool: $tool" >&2
      exit 1
    fi
  done
}

iso_in_seconds() {
  local offset="$1"
  if date -u -v+"${offset}S" +%Y-%m-%dT%H:%M:%SZ >/dev/null 2>&1; then
    date -u -v+"${offset}S" +%Y-%m-%dT%H:%M:%SZ
  else
    date -u -d "+${offset} seconds" +%Y-%m-%dT%H:%M:%SZ
  fi
}

unique_key() {
  echo "e2e-$(date +%s)-$RANDOM"
}

assert_eq() {
  local actual="$1"
  local expected="$2"
  local message="${3:-values differ}"
  if [[ "$actual" != "$expected" ]]; then
    echo "ASSERT FAILED: $message" >&2
    echo "  expected: $expected" >&2
    echo "  actual:   $actual" >&2
    return 1
  fi
}

assert_http_status() {
  local expected="$1"
  local actual="$2"
  local message="${3:-unexpected HTTP status}"
  if [[ "$actual" != "$expected" ]]; then
    echo "ASSERT FAILED: $message (expected $expected, got $actual)" >&2
    return 1
  fi
}

# Parse a scalar Prometheus sample (e.g. jobs_completed_total 3).
metric_value() {
  local name="$1"
  local metrics="$2"
  echo "$metrics" | awk -v m="$name" '
    $0 !~ /^#/ && $1 == m { print $2; found = 1; exit }
    END { if (!found) print "" }
  '
}

assert_metric_gte() {
  local name="$1"
  local metrics="$2"
  local minimum="$3"
  local value
  value=$(metric_value "$name" "$metrics")
  if [[ -z "$value" ]]; then
    echo "ASSERT FAILED: metric $name not found in /metrics output" >&2
    return 1
  fi
  if ! awk -v v="$value" -v m="$minimum" 'BEGIN { exit (v + 0 >= m + 0) ? 0 : 1 }'; then
    echo "ASSERT FAILED: $name=$value expected >= $minimum" >&2
    return 1
  fi
}

assert_metric_increased() {
  local name="$1"
  local before_metrics="$2"
  local after_metrics="$3"
  local before after
  before=$(metric_value "$name" "$before_metrics")
  before=${before:-0}
  after=$(metric_value "$name" "$after_metrics")
  if [[ -z "$after" ]]; then
    echo "ASSERT FAILED: metric $name missing after job run" >&2
    return 1
  fi
  if ! awk -v b="$before" -v a="$after" 'BEGIN { exit (a + 0 > b + 0) ? 0 : 1 }'; then
    echo "ASSERT FAILED: $name did not increase (before=$before after=$after)" >&2
    return 1
  fi
}

wait_for_metrics_ready() {
  local deadline=$((SECONDS + 30))
  while (( SECONDS < deadline )); do
    local metrics
    metrics=$(fetch_metrics)
    if echo "$metrics" | grep -q "scheduler_heap_size"; then
      echo "$metrics"
      return 0
    fi
    sleep 1
  done
  echo "Timed out waiting for Prometheus metrics to become ready" >&2
  return 1
}

fetch_metrics() {
  curl -sf "$BASE_URL/metrics" -H "X-API-Key: $API_KEY"
}

assert_metrics_has_type() {
  local name="$1"
  local kind="$2"
  local metrics="$3"
  echo "$metrics" | grep -q "TYPE ${name} ${kind}" || {
    echo "ASSERT FAILED: expected TYPE ${name} ${kind} in /metrics" >&2
    return 1
  }
}

run_test() {
  local name="$1"
  shift
  TESTS_RUN=$((TESTS_RUN + 1))
  echo ""
  echo "== TEST: $name =="
  if "$@"; then
    TESTS_PASSED=$((TESTS_PASSED + 1))
    echo "PASS: $name"
  else
    echo "FAIL: $name" >&2
    return 1
  fi
}

wait_for_service() {
  local url="$1"
  local label="$2"
  local attempts="${3:-60}"
  echo "Waiting for $label at $url ..."
  for _ in $(seq 1 "$attempts"); do
    if curl -sf "$url" >/dev/null 2>&1; then
      echo "$label is up"
      return 0
    fi
    sleep 2
  done
  echo "Timed out waiting for $label" >&2
  return 1
}

wait_for_job_state() {
  local job_id="$1"
  local expected_state="$2"
  local timeout_secs="${3:-90}"
  local deadline=$((SECONDS + timeout_secs))

  while (( SECONDS < deadline )); do
    local state
    state=$(curl -sf "$BASE_URL/api/v1/jobs/$job_id" \
      -H "X-API-Key: $API_KEY" | jq -r .state)
    if [[ "$state" == "$expected_state" ]]; then
      echo "Job $job_id reached $expected_state"
      return 0
    fi
    sleep 2
  done

  local final
  final=$(curl -sf "$BASE_URL/api/v1/jobs/$job_id" \
    -H "X-API-Key: $API_KEY" | jq -r .state)
  echo "Timed out waiting for job $job_id to reach $expected_state (last state: $final)" >&2
  return 1
}

create_job() {
  local body="$1"
  curl -sf -X POST "$BASE_URL/api/v1/jobs" \
    -H "X-API-Key: $API_KEY" \
    -H "Content-Type: application/json" \
    -d "$body"
}

create_job_expect_status() {
  local expected_status="$1"
  local body="$2"
  local response
  response=$(curl -s -w "\n%{http_code}" -X POST "$BASE_URL/api/v1/jobs" \
    -H "X-API-Key: $API_KEY" \
    -H "Content-Type: application/json" \
    -d "$body")
  local status
  status=$(echo "$response" | tail -n1)
  local payload
  payload=$(echo "$response" | sed '$d')
  assert_http_status "$expected_status" "$status" "create job"
  echo "$payload"
}

get_webhook_receipt_count() {
  docker compose -f "$COMPOSE_FILE" exec -T webhook-receiver \
    python -c "import urllib.request, json; print(json.load(urllib.request.urlopen('http://127.0.0.1:8080/receipts'))['count'])"
}

wait_for_webhook_receipts() {
  local min_count="$1"
  local timeout_secs="${2:-60}"
  local deadline=$((SECONDS + timeout_secs))

  while (( SECONDS < deadline )); do
    local count
    count=$(get_webhook_receipt_count)
    if (( count >= min_count )); then
      echo "Webhook receiver has $count receipt(s)"
      return 0
    fi
    sleep 2
  done

  echo "Timed out waiting for at least $min_count webhook receipt(s)" >&2
  return 1
}

start_stack() {
  cd "$ROOT"
  docker compose -f "$COMPOSE_FILE" down -v --remove-orphans 2>/dev/null || true
  docker compose -f "$COMPOSE_FILE" up --build -d
  wait_for_service "$BASE_URL/health" "kronkeeper API"
}

stop_stack() {
  cd "$ROOT"
  if [[ "${KEEP_E2E_STACK:-0}" == "1" ]]; then
    echo "KEEP_E2E_STACK=1 — leaving containers running"
    return 0
  fi
  docker compose -f "$COMPOSE_FILE" down -v --remove-orphans
}

print_summary() {
  echo ""
  echo "============================================"
  echo "E2E summary: $TESTS_PASSED / $TESTS_RUN passed"
  echo "============================================"
}
