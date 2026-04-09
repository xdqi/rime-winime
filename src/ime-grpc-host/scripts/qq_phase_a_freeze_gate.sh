#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
cd "${ROOT_DIR}"

mkdir -p .cache

PORT=${IME_GRPC_PORT:-50160}
READY_MAX_ATTEMPTS=${IME_READY_MAX_ATTEMPTS:-40}
CHECK_TIMEOUT_SEC=${IME_PHASE_A_CHECK_TIMEOUT_SEC:-30}
WORKER_BACKEND=${IME_WORKER_BACKEND:-stub}
CLEAN_STALE_HOSTS=${IME_CLEAN_STALE_HOSTS:-1}

STAMP=$(date +%Y%m%d_%H%M%S_%N)
RUN_LOG=".cache/qq-phase-a-gate-run-${STAMP}.log"
HOST_LOG=".cache/qq-phase-a-gate-host-${STAMP}.log"
CHECK_LOG=".cache/qq-phase-a-gate-check-${STAMP}.log"

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
  log_line "[summary] check_log=${CHECK_LOG}"
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

cleanup_host_log_tee() {
  set +e
  pkill -f "tee -a ${HOST_LOG}"
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

  cleanup_host_log_tee

  if [[ "${CLEAN_STALE_HOSTS}" == "1" ]]; then
    cleanup_stale_hosts
  fi
}
trap cleanup EXIT INT TERM

if [[ "${CLEAN_STALE_HOSTS}" == "1" ]]; then
  cleanup_stale_hosts
fi

if [[ ! -x "target/debug/phase_a_contract_checker" || ! -x "target/debug/ime-grpc-host" ]]; then
  log_line "[build] cargo build --bins"
  cargo build --bins 2>&1 | tee -a "${RUN_LOG}"
fi

SELECTED_PORT=$(pick_available_port "${PORT}" || true)
if [[ -z "${SELECTED_PORT}" ]]; then
  fail_with 10 "no_available_port"
fi
PORT="${SELECTED_PORT}"

log_line "[host] start port=${PORT} backend=${WORKER_BACKEND}"
IME_GRPC_BIND="127.0.0.1:${PORT}" IME_WORKER_BACKEND="${WORKER_BACKEND}" cargo run --bin ime-grpc-host > >(tee -a "${HOST_LOG}") 2>&1 &
HOST_PID=$!

if ! wait_for_listen_port "${PORT}"; then
  fail_with 11 "host_ready_timeout"
fi

log_line "[check] run phase_a_contract_checker"
timeout_path=$(command -v timeout || true)

set +e
if [[ -n "${timeout_path}" ]]; then
  env IME_GRPC_ENDPOINT="http://127.0.0.1:${PORT}" timeout --foreground "${CHECK_TIMEOUT_SEC}s" target/debug/phase_a_contract_checker 2>&1 | tee "${CHECK_LOG}"
  checker_rc=$?
else
  env IME_GRPC_ENDPOINT="http://127.0.0.1:${PORT}" target/debug/phase_a_contract_checker 2>&1 | tee "${CHECK_LOG}"
  checker_rc=$?
fi
set -e

if [[ ${checker_rc} -ne 0 ]]; then
  fail_with 20 "phase_a_checker_failed_rc_${checker_rc}"
fi

log_line "[result] PASS"
log_line "[summary] run_log=${RUN_LOG}"
log_line "[summary] host_log=${HOST_LOG}"
log_line "[summary] check_log=${CHECK_LOG}"
