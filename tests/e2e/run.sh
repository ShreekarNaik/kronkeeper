#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib.sh
source "$SCRIPT_DIR/lib.sh"

test_health() {
  local resp
  resp=$(curl -sf "$BASE_URL/health")
  assert_eq "$(echo "$resp" | jq -r .status)" "ok" "health status"
  assert_eq "$(echo "$resp" | jq -r .database)" "up" "database health"
  assert_eq "$(echo "$resp" | jq -r .scheduler)" "up" "scheduler health"
}

test_metrics() {
  local headers metrics before after key schedule resp job_id status

  status=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/metrics")
  assert_http_status "401" "$status" "metrics without API key"

  headers=$(curl -s -D - -o /dev/null "$BASE_URL/metrics" -H "X-API-Key: $API_KEY")
  echo "$headers" | grep -qi "HTTP/.* 200" || {
    echo "GET /metrics should return 200 with a valid API key" >&2
    return 1
  }
  echo "$headers" | grep -qi "content-type:.*text/plain" || {
    echo "GET /metrics should return text/plain Prometheus exposition format" >&2
    return 1
  }

  metrics=$(wait_for_metrics_ready)
  assert_metrics_has_type "scheduler_heap_size" "gauge" "$metrics"
  assert_metric_gte "scheduler_heap_size" "$metrics" 0

  before="$metrics"

  key=$(unique_key)
  schedule=$(iso_in_seconds 12)
  resp=$(create_job_expect_status 201 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "script",
    "path": "hello.sh",
    "timeout_sec": 30
  },
  "scheduled_at": "$schedule"
}
EOF
)")
  job_id=$(echo "$resp" | jq -r .id)
  wait_for_job_state "$job_id" "COMPLETED" 90

  after=$(fetch_metrics)

  assert_metrics_has_type "jobs_scheduled_total" "counter" "$after"
  assert_metrics_has_type "jobs_dispatched_total" "counter" "$after"
  assert_metrics_has_type "jobs_completed_total" "counter" "$after"
  assert_metrics_has_type "job_execution_duration_seconds" "histogram" "$after"
  assert_metric_increased "jobs_scheduled_total" "$before" "$after"
  assert_metric_increased "jobs_dispatched_total" "$before" "$after"
  assert_metric_increased "jobs_completed_total" "$before" "$after"
  assert_metric_gte "job_execution_duration_seconds_count" "$after" 1

  local active
  active=$(metric_value "worker_active_count" "$after")
  if [[ -n "$active" ]]; then
    assert_eq "$active" "0" "worker_active_count after job completes"
  fi

  echo "Metrics snapshot after job run:"
  echo "$after" | grep -E '^(jobs_|scheduler_|worker_|job_execution_duration_seconds_)' || true
}

test_auth() {
  local status
  status=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/api/v1/jobs")
  assert_http_status "401" "$status" "missing API key on jobs"

  status=$(curl -s -o /dev/null -w "%{http_code}" "$BASE_URL/metrics")
  assert_http_status "401" "$status" "missing API key on metrics"

  status=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "X-API-Key: not-a-real-key" \
    "$BASE_URL/api/v1/jobs")
  assert_http_status "401" "$status" "invalid API key on jobs"

  status=$(curl -s -o /dev/null -w "%{http_code}" \
    -H "X-API-Key: not-a-real-key" \
    "$BASE_URL/metrics")
  assert_http_status "401" "$status" "invalid API key on metrics"
}

test_script_job_completes() {
  local key schedule job_id state
  key=$(unique_key)
  schedule=$(iso_in_seconds 12)

  local resp
  resp=$(create_job_expect_status 201 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "script",
    "path": "hello.sh",
    "timeout_sec": 30
  },
  "scheduled_at": "$schedule"
}
EOF
)")
  job_id=$(echo "$resp" | jq -r .id)
  assert_eq "$(echo "$resp" | jq -r .state)" "SCHEDULED" "initial state"

  wait_for_job_state "$job_id" "COMPLETED" 90
  state=$(curl -sf "$BASE_URL/api/v1/jobs/$job_id" -H "X-API-Key: $API_KEY" | jq -r .state)
  assert_eq "$state" "COMPLETED" "final script job state"
}

test_http_job_completes() {
  local key schedule job_id
  key=$(unique_key)
  schedule=$(iso_in_seconds 12)

  local resp
  resp=$(create_job_expect_status 201 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "http",
    "method": "GET",
    "url": "${HTTP_TARGET_BASE}/echo",
    "timeout_sec": 10
  },
  "scheduled_at": "$schedule"
}
EOF
)")
  job_id=$(echo "$resp" | jq -r .id)
  wait_for_job_state "$job_id" "COMPLETED" 90
}

test_webhook_delivery() {
  local key schedule job_id before after
  key=$(unique_key)
  schedule=$(iso_in_seconds 12)

  before=$(get_webhook_receipt_count)

  local resp
  resp=$(create_job_expect_status 201 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "script",
    "path": "hello.sh",
    "timeout_sec": 30
  },
  "scheduled_at": "$schedule",
  "webhook_url": "$WEBHOOK_URL"
}
EOF
)")
  job_id=$(echo "$resp" | jq -r .id)
  wait_for_job_state "$job_id" "COMPLETED" 90
  wait_for_webhook_receipts "$((before + 1))" 30

  after=$(get_webhook_receipt_count)
  if (( after <= before )); then
    echo "expected webhook receipt after job completion" >&2
    return 1
  fi
}

test_idempotency() {
  local key schedule first_id second_status second_id
  key=$(unique_key)
  schedule=$(iso_in_seconds 120)

  local first
  first=$(create_job_expect_status 201 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "script",
    "path": "hello.sh",
    "timeout_sec": 30
  },
  "scheduled_at": "$schedule"
}
EOF
)")
  first_id=$(echo "$first" | jq -r .id)

  local second
  second=$(create_job_expect_status 200 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "script",
    "path": "hello.sh",
    "timeout_sec": 30
  },
  "scheduled_at": "$schedule"
}
EOF
)")
  second_id=$(echo "$second" | jq -r .id)
  assert_eq "$first_id" "$second_id" "idempotent job id"
}

test_cancel_scheduled_job() {
  local key schedule job_id state
  key=$(unique_key)
  schedule=$(iso_in_seconds 300)

  local resp
  resp=$(create_job_expect_status 201 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "script",
    "path": "hello.sh",
    "timeout_sec": 30
  },
  "scheduled_at": "$schedule"
}
EOF
)")
  job_id=$(echo "$resp" | jq -r .id)

  local status
  status=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE \
    "$BASE_URL/api/v1/jobs/$job_id" \
    -H "X-API-Key: $API_KEY")
  assert_http_status "204" "$status" "cancel job"

  state=$(curl -sf "$BASE_URL/api/v1/jobs/$job_id" -H "X-API-Key: $API_KEY" | jq -r .state)
  assert_eq "$state" "CANCELLED" "cancelled job state"
}

test_failed_job_dead_letter() {
  local key schedule job_id
  key=$(unique_key)
  schedule=$(iso_in_seconds 10)

  local resp
  resp=$(create_job_expect_status 201 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "script",
    "path": "fail.sh",
    "timeout_sec": 10
  },
  "scheduled_at": "$schedule",
  "max_retries": 0,
  "retry_delay_sec": 1
}
EOF
)")
  job_id=$(echo "$resp" | jq -r .id)
  wait_for_job_state "$job_id" "DEAD_LETTER" 90
}

test_recurring_job_lifecycle() {
  local key schedule template_id instance_id patched_cron
  key=$(unique_key)
  schedule=$(iso_in_seconds 15)

  local resp
  resp=$(create_job_expect_status 201 "$(cat <<EOF
{
  "idempotency_key": "$key",
  "payload": {
    "type": "script",
    "path": "hello.sh",
    "timeout_sec": 30
  },
  "scheduled_at": "$schedule",
  "recurrence": {
    "cron_expr": "0 2 * * *",
    "max_occurrences": 3,
    "concurrency_policy": "queue_once"
  }
}
EOF
)")
  instance_id=$(echo "$resp" | jq -r .id)
  assert_eq "$(echo "$resp" | jq -r .is_template)" "false" "create returns instance"

  template_id=$(curl -sf "$BASE_URL/api/v1/jobs/$instance_id" \
    -H "X-API-Key: $API_KEY" | jq -r .parent_job_id)
  [[ -n "$template_id" && "$template_id" != "null" ]] || {
    echo "expected parent template id on recurring instance" >&2
    return 1
  }

  local template
  template=$(curl -sf "$BASE_URL/api/v1/jobs/$template_id" \
    -H "X-API-Key: $API_KEY")
  assert_eq "$(echo "$template" | jq -r .is_template)" "true" "template flag"
  assert_eq "$(echo "$template" | jq -r .is_recurring)" "true" "recurring flag"

  patched_cron="0 3 * * *"
  local patched
  patched=$(curl -sf -X PATCH "$BASE_URL/api/v1/jobs/$template_id" \
    -H "X-API-Key: $API_KEY" \
    -H "Content-Type: application/json" \
    -d "{\"cron_expr\": \"$patched_cron\"}")
  assert_eq "$(echo "$patched" | jq -r .cron_expr)" "$patched_cron" "patched cron"

  local status
  status=$(curl -s -o /dev/null -w "%{http_code}" -X DELETE \
    "$BASE_URL/api/v1/jobs/$template_id" \
    -H "X-API-Key: $API_KEY")
  assert_http_status "204" "$status" "cancel recurring template"

  local instance_state template_state
  instance_state=$(curl -sf "$BASE_URL/api/v1/jobs/$instance_id" \
    -H "X-API-Key: $API_KEY" | jq -r .state)
  template_state=$(curl -sf "$BASE_URL/api/v1/jobs/$template_id" \
    -H "X-API-Key: $API_KEY" | jq -r .state)
  assert_eq "$instance_state" "CANCELLED" "instance cancelled with template"
  assert_eq "$template_state" "CANCELLED" "template cancelled"
}

main() {
  require_tools
  chmod +x "$ROOT/scripts/hello.sh" "$ROOT/scripts/fail.sh" 2>/dev/null || true

  start_stack

  run_test "health endpoint" test_health
  run_test "prometheus metrics endpoint" test_metrics
  run_test "API key authentication" test_auth
  run_test "scheduled script job completes" test_script_job_completes
  run_test "scheduled HTTP job completes" test_http_job_completes
  run_test "webhook delivery on completion" test_webhook_delivery
  run_test "idempotency key deduplication" test_idempotency
  run_test "cancel scheduled job" test_cancel_scheduled_job
  run_test "failed job reaches dead letter" test_failed_job_dead_letter
  run_test "recurring job lifecycle" test_recurring_job_lifecycle

  stop_stack
  print_summary

  if [[ "$TESTS_PASSED" -ne "$TESTS_RUN" ]]; then
    exit 1
  fi
}

main "$@"
