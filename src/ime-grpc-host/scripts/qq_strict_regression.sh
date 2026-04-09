#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
cd "${ROOT_DIR}"

PORT=${IME_GRPC_PORT:-50096}
WINE_PREFIX=${WINEPREFIX:-/opt/sogou/.wine32}
DLL_PATH=${IME_WINIMM_DLL:-C:\\windows\\system32\\QQPinyin.ime}
READY_MAX_ATTEMPTS=${IME_READY_MAX_ATTEMPTS:-40}
SMOKE_MAX_ATTEMPTS=${IME_SMOKE_MAX_ATTEMPTS:-8}
SMOKE_INPUTS=${IME_SMOKE_INPUTS:-"nihao rime"}
RPC_TIMEOUT_SEC=${IME_RPC_TIMEOUT_SEC:-20}
CLEAN_STALE_HOSTS=${IME_CLEAN_STALE_HOSTS:-1}
STAMP=$(date +%Y%m%d_%H%M%S)

HOST_LOG=".cache/qq-strict-host-${STAMP}.log"
RUN_LOG=".cache/qq-strict-run-${STAMP}.log"
READY_LOG=".cache/qq-strict-ready-${STAMP}.log"
SMOKE_NIHAO_LOG=".cache/qq-strict-smoke-nihao-${STAMP}.log"
SMOKE_RIME_LOG=".cache/qq-strict-smoke-rime-${STAMP}.log"
SMOKE_V123_LOG=".cache/qq-strict-smoke-v123-${STAMP}.log"

mkdir -p .cache

log_line() {
  echo "$*" | tee -a "${RUN_LOG}"
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

  if [[ "${IME_GRPC_FAIL_ON_BUSY:-0}" == "1" ]]; then
    return 1
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
  pkill -f "target/i686-pc-windows-gnu/debug/ime-grpc-host.exe"
  pkill -f "ime-grpc-host.exe --worker-runtime"
  set -e
}

cleanup_host_log_tee() {
  set +e
  pkill -f "tee -a ${HOST_LOG}"
  set -e
}

wait_for_host_ready() {
  local attempt=1
  local max_attempts=${READY_MAX_ATTEMPTS}

  while [[ ${attempt} -le ${max_attempts} ]]; do
    log_line "[host] readiness attempt=${attempt}/${max_attempts}"

    if ! kill -0 "${HOST_PID}"; then
      log_line "[host] exited before ready host_log=${HOST_LOG}"
      return 1
    fi

    set +e
    run_smoke_client "" "${READY_LOG}"
    local rc=$?
    set -e

    if [[ ${rc} -eq 0 ]]; then
      log_line "[host] readiness confirmed"
      return 0
    fi

    read -r -t 1 _ || true
    attempt=$((attempt + 1))
  done

  log_line "[host] readiness timeout host_log=${HOST_LOG} ready_log=${READY_LOG}"
  return 1
}

run_smoke_with_retry() {
  local input="$1"
  local out_log="$2"
  local attempt=1
  local max_attempts=${SMOKE_MAX_ATTEMPTS}

  while [[ ${attempt} -le ${max_attempts} ]]; do
    log_line "[smoke] input=${input} attempt=${attempt}/${max_attempts}"

    if ! kill -0 "${HOST_PID}"; then
      log_line "[smoke] host died before input=${input} host_log=${HOST_LOG}"
      return 1
    fi

    set +e
    run_smoke_client "${input}" "${out_log}"
    local rc=$?
    set -e

    if [[ ${rc} -eq 0 ]]; then
      return 0
    fi

    # Allow host startup and worker warmup without using external sleep.
    read -r -t 1 _ || true

    attempt=$((attempt + 1))
  done

  return 1
}

cleanup() {
  if [[ -n "${HOST_PID:-}" ]]; then
    if kill -0 "${HOST_PID}" 2>/dev/null; then
      kill "${HOST_PID}" 2>/dev/null || true

      if [[ "${CLEAN_STALE_HOSTS}" == "1" ]]; then
        cleanup_stale_hosts
      fi

      local tries=0
      while [[ ${tries} -lt 5 ]]; do
        if ! kill -0 "${HOST_PID}" 2>/dev/null; then
          break
        fi
        read -r -t 1 _ || true
        tries=$((tries + 1))
      done
      kill -9 "${HOST_PID}" 2>/dev/null || true

      if [[ "${CLEAN_STALE_HOSTS}" == "1" ]]; then
        cleanup_stale_hosts
      fi

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
  log_line "[host] cleanup stale processes before start"
  cleanup_stale_hosts
fi

SELECTED_PORT=$(pick_available_port "${PORT}" || true)
if [[ -z "${SELECTED_PORT}" ]]; then
  log_line "[host] no available port near ${PORT}; set IME_GRPC_PORT or stop conflicting hosts"
  exit 1
fi

if [[ "${SELECTED_PORT}" != "${PORT}" ]]; then
  log_line "[host] port ${PORT} busy; auto-selected ${SELECTED_PORT}"
fi
PORT=${SELECTED_PORT}

log_line "[host] start port=${PORT} dll=${DLL_PATH}"
WINEPREFIX="${WINE_PREFIX}" WINEDEBUG=-all RUST_LOG="${RUST_LOG:-}" IME_WINIMM_TRACE_TIMELINE="${IME_WINIMM_TRACE_TIMELINE:-0}" IME_GRPC_BIND="127.0.0.1:${PORT}" IME_WORKER_BACKEND=win_imm IME_WINIMM_FORCE_REAL=1 IME_WINIMM_DLL="${DLL_PATH}" wine target/i686-pc-windows-gnu/debug/ime-grpc-host.exe > >(tee -a "${HOST_LOG}") 2>&1 &
HOST_PID=$!

wait_for_host_ready

for input in ${SMOKE_INPUTS}; do
  case "${input}" in
    nihao)
      run_smoke_with_retry "${input}" "${SMOKE_NIHAO_LOG}"
      ;;
    rime)
      run_smoke_with_retry "${input}" "${SMOKE_RIME_LOG}"
      ;;
    v123)
      run_smoke_with_retry "${input}" "${SMOKE_V123_LOG}"
      ;;
    *)
      run_smoke_with_retry "${input}" ".cache/qq-strict-smoke-${input}-${STAMP}.log"
      ;;
  esac
done

log_line "[summary] logs:"
log_line "[summary] host=${HOST_LOG}"
log_line "[summary] ready=${READY_LOG}"
log_line "[summary] nihao=${SMOKE_NIHAO_LOG}"
log_line "[summary] rime=${SMOKE_RIME_LOG}"
log_line "[summary] v123=${SMOKE_V123_LOG}"
log_line "[summary] run=${RUN_LOG}"
