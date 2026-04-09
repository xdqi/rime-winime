#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
cd "${ROOT_DIR}"

mkdir -p .cache

BASE_PORT=${IME_GRPC_PORT:-50140}
READY_MAX_ATTEMPTS=${IME_READY_MAX_ATTEMPTS:-40}
WORKER_BACKEND=${IME_WORKER_BACKEND:-stub}
BENCH_ROUNDS=${IME_BENCH_ROUNDS:-1000}
BENCH_INPUT=${IME_BENCH_INPUT:-nihao}
TARGET_P95_MS=${IME_LATENCY_TARGET_P95_MS:-10}
ENFORCE_TARGET=${IME_LATENCY_ENFORCE_TARGET:-0}
CLEAN_STALE_HOSTS=${IME_CLEAN_STALE_HOSTS:-1}

STAMP=$(date +%Y%m%d_%H%M%S_%N)
RUN_LOG=".cache/qq-latency-benchmark-run-${STAMP}.log"

HOST_PID=""

log_line() {
  echo "$*" | tee -a "${RUN_LOG}"
}

fail_with() {
  local code="$1"
  local reason="$2"
  log_line "[result] FAIL code=${code} reason=${reason}"
  log_line "[summary] run_log=${RUN_LOG}"
  exit "${code}"
}

is_port_busy() {
  local port="$1"

  local ss_path=""
  ss_path=$(command -v ss || true)
  if [[ -n "${ss_path}" ]]; then
    if ss -ltn | awk -v port=":${port}" '$4 ~ port"$" { found=1 } END { exit(found ? 0 : 1) }'; then
      return 0
    fi
  fi

  local netstat_path=""
  netstat_path=$(command -v netstat || true)
  if [[ -n "${netstat_path}" ]]; then
    if netstat -ltn | awk -v port=":${port}" '$4 ~ port"$" { found=1 } END { exit(found ? 0 : 1) }'; then
      return 0
    fi
  fi

  return 1
}

pick_available_port() {
  local preferred="$1"

  if ! is_port_busy "${preferred}"; then
    echo "${preferred}"
    return 0
  fi

  local candidate=$((preferred + 1))
  local end=$((preferred + 120))
  while [[ ${candidate} -le ${end} ]]; do
    if ! is_port_busy "${candidate}"; then
      echo "${candidate}"
      return 0
    fi
    candidate=$((candidate + 1))
  done

  return 1
}

wait_for_listen_port() {
  local endpoint_port="$1"
  local attempt=1

  while [[ ${attempt} -le ${READY_MAX_ATTEMPTS} ]]; do
    log_line "[host] listen readiness attempt=${attempt}/${READY_MAX_ATTEMPTS} port=${endpoint_port}"

    if ! kill -0 "${HOST_PID}" 2>/dev/null; then
      return 1
    fi

    if is_port_busy "${endpoint_port}"; then
      return 0
    fi

    read -r -t 1 _ || true
    attempt=$((attempt + 1))
  done

  return 1
}

cleanup_stale_hosts() {
  set +e
  pkill -f "cargo run --bin ime-grpc-host"
  pkill -f "target/debug/ime-grpc-host"
  set -e
}

stop_host() {
  if [[ -n "${HOST_PID:-}" ]]; then
    if kill -0 "${HOST_PID}" 2>/dev/null; then
      kill "${HOST_PID}" 2>/dev/null || true
      local tries=0
      while [[ ${tries} -lt 5 ]]; do
        if ! kill -0 "${HOST_PID}" 2>/dev/null; then
          break
        fi
        read -r -t 1 _ || true
        tries=$((tries + 1))
      done
      kill -9 "${HOST_PID}" 2>/dev/null || true
      wait "${HOST_PID}" 2>/dev/null || true
    fi
    HOST_PID=""
  fi
}

cleanup() {
  stop_host
  if [[ "${CLEAN_STALE_HOSTS}" == "1" ]]; then
    cleanup_stale_hosts
  fi
}
trap cleanup EXIT INT TERM

if [[ "${CLEAN_STALE_HOSTS}" == "1" ]]; then
  cleanup_stale_hosts
fi

if [[ ! -x "target/debug/ime-grpc-host" || ! -x "target/debug/smoke_client" || ! -x "target/debug/latency_probe" ]]; then
  log_line "[build] cargo build --bins"
  cargo build --bins 2>&1 | tee -a "${RUN_LOG}"
fi

CASE_P50=""
CASE_P95=""
CASE_P99=""
CASE_BENCH_LOG=""

extract_metric() {
  local metric="$1"
  local log_file="$2"
  awk -v metric="${metric}" '/^latency_stats / { line=$0 } END { if (line == "") exit 1; n=split(line, parts, " "); for (i=1; i<=n; i++) { if (parts[i] ~ ("^" metric "=")) { split(parts[i], kv, "="); print kv[2]; exit 0 } } exit 1 }' "${log_file}"
}

run_case() {
  local case_name="$1"
  local prefer_port="$2"
  local pool_min_idle="$3"
  local pool_prewarm="$4"
  local want_prewarmed="$5"

  local case_host_log=".cache/qq-latency-host-${case_name}-${STAMP}.log"
  local case_bench_log=".cache/qq-latency-bench-${case_name}-${STAMP}.log"

  local selected_port=""
  selected_port=$(pick_available_port "${prefer_port}" || true)
  if [[ -z "${selected_port}" ]]; then
    fail_with 20 "no_available_port_${case_name}"
  fi

  log_line "[case:${case_name}] start port=${selected_port} pool_min_idle=${pool_min_idle} pool_prewarm=${pool_prewarm} want_prewarmed=${want_prewarmed}"
  IME_GRPC_BIND="127.0.0.1:${selected_port}" \
  IME_WORKER_BACKEND="${WORKER_BACKEND}" \
  IME_POOL_MIN_IDLE="${pool_min_idle}" \
  IME_POOL_PREWARM="${pool_prewarm}" \
  cargo run --bin ime-grpc-host > >(tee -a "${case_host_log}") 2>&1 &
  HOST_PID=$!

  if ! wait_for_listen_port "${selected_port}"; then
    fail_with 22 "host_ready_timeout_${case_name}"
  fi

  log_line "[case:${case_name}] benchmark rounds=${BENCH_ROUNDS} input=${BENCH_INPUT}"
  env IME_GRPC_ENDPOINT="http://127.0.0.1:${selected_port}" \
    IME_BENCH_ROUNDS="${BENCH_ROUNDS}" \
    IME_BENCH_INPUT="${BENCH_INPUT}" \
    IME_BENCH_WANT_PREWARMED="${want_prewarmed}" \
    target/debug/latency_probe 2>&1 | tee "${case_bench_log}"

  local p50=""
  local p95=""
  local p99=""
  p50=$(extract_metric "p50_ms" "${case_bench_log}" || true)
  p95=$(extract_metric "p95_ms" "${case_bench_log}" || true)
  p99=$(extract_metric "p99_ms" "${case_bench_log}" || true)

  if [[ -z "${p50}" || -z "${p95}" || -z "${p99}" ]]; then
    fail_with 23 "metric_parse_failed_${case_name}"
  fi

  log_line "[case:${case_name}] p50_ms=${p50} p95_ms=${p95} p99_ms=${p99}"
  log_line "[case:${case_name}] bench_log=${case_bench_log}"

  stop_host

  CASE_P50="${p50}"
  CASE_P95="${p95}"
  CASE_P99="${p99}"
  CASE_BENCH_LOG="${case_bench_log}"
}

run_case "cold" "${BASE_PORT}" 0 false false
cold_p50="${CASE_P50}"
cold_p95="${CASE_P95}"
cold_p99="${CASE_P99}"
cold_log="${CASE_BENCH_LOG}"

run_case "prewarm" "$((BASE_PORT + 1))" 1 true true
prewarm_p50="${CASE_P50}"
prewarm_p95="${CASE_P95}"
prewarm_p99="${CASE_P99}"
prewarm_log="${CASE_BENCH_LOG}"

log_line "[summary] cold_p50_ms=${cold_p50} cold_p95_ms=${cold_p95} cold_p99_ms=${cold_p99}"
log_line "[summary] prewarm_p50_ms=${prewarm_p50} prewarm_p95_ms=${prewarm_p95} prewarm_p99_ms=${prewarm_p99}"
log_line "[summary] target_p95_ms=${TARGET_P95_MS}"
log_line "[summary] cold_log=${cold_log}"
log_line "[summary] prewarm_log=${prewarm_log}"
log_line "[summary] run_log=${RUN_LOG}"

if [[ "${ENFORCE_TARGET}" == "1" ]]; then
  awk -v p95="${prewarm_p95}" -v target="${TARGET_P95_MS}" 'BEGIN { if ((p95 + 0) > (target + 0)) exit 1 }' || fail_with 24 "prewarm_p95_above_target"
fi

log_line "[result] PASS"
