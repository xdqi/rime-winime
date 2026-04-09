#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
cd "${ROOT_DIR}"

mkdir -p .cache

PORT=${IME_GRPC_PORT:-50120}
READY_MAX_ATTEMPTS=${IME_READY_MAX_ATTEMPTS:-40}
RPC_TIMEOUT_SEC=${IME_RPC_TIMEOUT_SEC:-20}
CLIENTS=${IME_CONCURRENCY_CLIENTS:-3}
INPUTS=${IME_CONCURRENCY_INPUTS:-"nihao rime abc"}
WORKER_BACKEND=${IME_WORKER_BACKEND:-stub}
CLEAN_STALE_HOSTS=${IME_CLEAN_STALE_HOSTS:-1}

STAMP=$(date +%Y%m%d_%H%M%S_%N)
HOST_LOG=".cache/qq-concurrency-host-${STAMP}.log"
RUN_LOG=".cache/qq-concurrency-run-${STAMP}.log"
READY_LOG=".cache/qq-concurrency-ready-${STAMP}.log"

HOST_PID=""

log_line() {
  echo "$*" | tee -a "${RUN_LOG}"
}

fail_with() {
  local code="$1"
  local reason="$2"
  log_line "[result] FAIL code=${code} reason=${reason}"
  log_line "[summary] run_log=${RUN_LOG}"
  log_line "[summary] host_log=${HOST_LOG}"
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

run_smoke_client() {
  local input="$1"
  local out_log="$2"
  local timeout_path=""
  timeout_path=$(command -v timeout || true)

  if [[ -n "${timeout_path}" ]]; then
    env IME_GRPC_ENDPOINT="http://127.0.0.1:${PORT}" IME_SMOKE_INPUT="${input}" timeout --foreground "${RPC_TIMEOUT_SEC}s" target/debug/smoke_client 2>&1 | tee -a "${out_log}"
  else
    env IME_GRPC_ENDPOINT="http://127.0.0.1:${PORT}" IME_SMOKE_INPUT="${input}" target/debug/smoke_client 2>&1 | tee -a "${out_log}"
  fi
}

cleanup_stale_hosts() {
  set +e
  pkill -f "cargo run --bin ime-grpc-host"
  pkill -f "target/debug/ime-grpc-host"
  set -e
}

cleanup() {
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
  fi

  if [[ "${CLEAN_STALE_HOSTS}" == "1" ]]; then
    cleanup_stale_hosts
  fi
}
trap cleanup EXIT INT TERM

if [[ "${CLEAN_STALE_HOSTS}" == "1" ]]; then
  cleanup_stale_hosts
fi

if [[ ! -x "target/debug/smoke_client" || ! -x "target/debug/ime-grpc-host" ]]; then
  log_line "[build] cargo build --bins"
  cargo build --bins 2>&1 | tee -a "${RUN_LOG}"
fi

SELECTED_PORT=$(pick_available_port "${PORT}" || true)
if [[ -z "${SELECTED_PORT}" ]]; then
  fail_with 10 "no_available_port"
fi
PORT=${SELECTED_PORT}

log_line "[host] start port=${PORT} backend=${WORKER_BACKEND}"
IME_GRPC_BIND="127.0.0.1:${PORT}" IME_WORKER_BACKEND="${WORKER_BACKEND}" cargo run --bin ime-grpc-host > >(tee -a "${HOST_LOG}") 2>&1 &
HOST_PID=$!

attempt=1
while [[ ${attempt} -le ${READY_MAX_ATTEMPTS} ]]; do
  log_line "[host] readiness attempt=${attempt}/${READY_MAX_ATTEMPTS}"

  if ! kill -0 "${HOST_PID}" 2>/dev/null; then
    fail_with 11 "host_exited_before_ready"
  fi

  set +e
  run_smoke_client "" "${READY_LOG}"
  ready_rc=$?
  set -e

  if [[ ${ready_rc} -eq 0 ]]; then
    break
  fi

  read -r -t 1 _ || true
  attempt=$((attempt + 1))
done

if [[ ${attempt} -gt ${READY_MAX_ATTEMPTS} ]]; then
  fail_with 12 "host_ready_timeout"
fi

read -r -a input_array <<< "${INPUTS}"
if [[ ${#input_array[@]} -eq 0 ]]; then
  fail_with 13 "empty_inputs"
fi

declare -a pids=()
declare -a logs=()

client=1
while [[ ${client} -le ${CLIENTS} ]]; do
  idx=$(( (client - 1) % ${#input_array[@]} ))
  input="${input_array[${idx}]}"
  log_file=".cache/qq-concurrency-client-${client}-${STAMP}.log"
  logs+=("${log_file}")

  log_line "[client] start id=${client} input=${input}"
  env IME_GRPC_ENDPOINT="http://127.0.0.1:${PORT}" IME_SMOKE_INPUT="${input}" target/debug/smoke_client 2>&1 | tee "${log_file}" &
  pids+=("$!")

  client=$((client + 1))
done

for pid in "${pids[@]}"; do
  set +e
  wait "${pid}"
  rc=$?
  set -e
  if [[ ${rc} -ne 0 ]]; then
    fail_with 14 "client_failed_rc_${rc}"
  fi
done

declare -a session_ids=()
for log_file in "${logs[@]}"; do
  sid=$(awk -F'session_id=' '/^open_session:/ { split($2, a, " "); print a[1]; exit }' "${log_file}")
  if [[ -z "${sid}" ]]; then
    fail_with 15 "missing_session_id_${log_file}"
  fi
  session_ids+=("${sid}")

  cand_count=$(awk '/^query_candidates:/ { if (match($0, /count=[0-9]+/)) { print substr($0, RSTART + 6, RLENGTH - 6); exit } }' "${log_file}")
  if [[ -z "${cand_count}" || ${cand_count} -le 0 ]]; then
    fail_with 16 "candidate_count_zero_${log_file}"
  fi

  non_empty_error=$(awk -F"error_code='" '/error_code=/{ if (NF > 1) { split($2, a, "\x27"); if (a[1] != "") { print a[1]; exit } } }' "${log_file}")
  if [[ -n "${non_empty_error}" ]]; then
    fail_with 17 "non_empty_error_code_${non_empty_error}"
  fi
done

unique_sessions=$(printf '%s\n' "${session_ids[@]}" | sort -u | wc -l | tr -d ' ')
if [[ "${unique_sessions}" != "${CLIENTS}" ]]; then
  fail_with 18 "session_collision_detected"
fi

log_line "[result] PASS"
log_line "[summary] clients=${CLIENTS} unique_sessions=${unique_sessions}"
log_line "[summary] run_log=${RUN_LOG}"
log_line "[summary] host_log=${HOST_LOG}"
