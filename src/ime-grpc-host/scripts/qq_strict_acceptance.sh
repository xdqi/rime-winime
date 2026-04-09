#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
PROXY_DIR=$(cd "${ROOT_DIR}/../rime-grpc-proxy" && pwd)
cd "${ROOT_DIR}"

mkdir -p .cache
mkdir -p "${PROXY_DIR}/.cache"

STAMP=$(date +%Y%m%d_%H%M%S_%N)
ACCEPT_LOG=".cache/qq-strict-acceptance-${STAMP}.log"
REGRESSION_LOG=".cache/qq-strict-acceptance-regression-${STAMP}.log"
REPLAY_HOST_LOG=".cache/qq-strict-acceptance-replay-host-${STAMP}.log"
REPLAY_LOG="${PROXY_DIR}/.cache/qq-strict-acceptance-replay-${STAMP}.log"

REPLAY_READY_MAX_ATTEMPTS=${IME_REPLAY_READY_MAX_ATTEMPTS:-60}
REPLAY_MAX_ATTEMPTS=${IME_REPLAY_MAX_ATTEMPTS:-5}
REPLAY_RPC_TIMEOUT_MS=${IME_REPLAY_RPC_TIMEOUT_MS:-8000}
REPLAY_INPUT=${IME_REPLAY_INPUT:-nihao}
REPLAY_BASE_PORT=${IME_REPLAY_PORT:-50106}
REPLAY_POOL_MIN_IDLE=${IME_REPLAY_POOL_MIN_IDLE:-1}
WINE_PREFIX=${WINEPREFIX:-/opt/sogou/.wine32}
DLL_PATH=${IME_WINIMM_DLL:-C:\\windows\\system32\\QQPinyin.ime}
PHASE1_GATE_PRECHECK=${IME_PHASE1_GATE_PRECHECK:-1}

REPLAY_HOST_PID=""
REPLAY_PORT=""
REGRESSION_PORT=""
PHASE1_GATE_LOG=".cache/qq-strict-acceptance-phase1-gate-${STAMP}.log"

log_line() {
  echo "$*" | tee -a "${ACCEPT_LOG}"
}

fail_with() {
  local code="$1"
  local reason="$2"
  log_line "[result] FAIL code=${code} reason=${reason}"
  log_line "[summary] acceptance_log=${ACCEPT_LOG}"
  log_line "[summary] phase1_gate_log=${PHASE1_GATE_LOG}"
  log_line "[summary] regression_log=${REGRESSION_LOG}"
  log_line "[summary] replay_log=${REPLAY_LOG}"
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

cleanup_stale_hosts() {
  set +e
  pkill -f "target/i686-pc-windows-gnu/debug/ime-grpc-host.exe"
  pkill -f "ime-grpc-host.exe --worker-runtime"
  set -e
}

cleanup_replay_host_log_tee() {
  set +e
  pkill -f "tee -a ${REPLAY_HOST_LOG}"
  set -e
}

cleanup() {
  if [[ -n "${REPLAY_HOST_PID:-}" ]]; then
    if kill -0 "${REPLAY_HOST_PID}" 2>/dev/null; then
      kill "${REPLAY_HOST_PID}" 2>/dev/null || true

      cleanup_stale_hosts

      local tries=0
      while [[ ${tries} -lt 5 ]]; do
        if ! kill -0 "${REPLAY_HOST_PID}" 2>/dev/null; then
          break
        fi
        read -r -t 1 _ || true
        tries=$((tries + 1))
      done
      kill -9 "${REPLAY_HOST_PID}" 2>/dev/null || true

      cleanup_stale_hosts

      wait "${REPLAY_HOST_PID}" 2>/dev/null || true
    fi
  fi

  cleanup_replay_host_log_tee

  cleanup_stale_hosts
}
trap cleanup EXIT INT TERM

extract_summary_path() {
  local key="$1"
  local source_log="$2"
  awk -v key="${key}" '$0 ~ "^\\[summary\\] " key "=" { sub("^\\[summary\\] " key "=", "", $0); value=$0 } END { if (value != "") print value }' "${source_log}"
}

log_line "[phase] precheck start"
if [[ "${PHASE1_GATE_PRECHECK}" == "1" ]]; then
  log_line "[phase] phase1_gate_winimm start"
  set +e
  ./scripts/qq_phase1_gate_winimm.sh 2>&1 | tee "${PHASE1_GATE_LOG}"
  phase1_rc=$?
  set -e
  if [[ ${phase1_rc} -ne 0 ]]; then
    fail_with 21 "phase1_gate_winimm_failed"
  fi
else
  log_line "[phase] phase1_gate_winimm skipped"
fi

log_line "[phase] regression start"
set +e
./scripts/qq_strict_regression.sh 2>&1 | tee "${REGRESSION_LOG}"
regression_rc=$?
set -e
if [[ ${regression_rc} -ne 0 ]]; then
  fail_with 10 "regression_failed"
fi

NIHAO_LOG=$(extract_summary_path "nihao" "${REGRESSION_LOG}")
if [[ -z "${NIHAO_LOG}" || ! -f "${NIHAO_LOG}" ]]; then
  fail_with 18 "summary_parse_failed_nihao"
fi

RUN_LOG=$(extract_summary_path "run" "${REGRESSION_LOG}")
if [[ -z "${RUN_LOG}" || ! -f "${RUN_LOG}" ]]; then
  fail_with 18 "summary_parse_failed_run"
fi

REGRESSION_PORT=$(awk '/^\[host\] start port=/ { line=$0 } END { if (line != "") { if (match(line, /port=[0-9]+/)) { value=substr(line, RSTART + 5, RLENGTH - 5); print value } } }' "${RUN_LOG}")

NIHAO_COUNT=$(awk '/query_candidates:/ { line=$0 } END { if (line == "") { print 0; exit } if (match(line, /count=[0-9]+/)) { value=substr(line, RSTART + 6, RLENGTH - 6); print value } else { print 0 } }' "${NIHAO_LOG}")
if [[ ${NIHAO_COUNT} -le 0 ]]; then
  fail_with 12 "nihao_candidate_count_zero"
fi

if ! grep -F "你好" "${NIHAO_LOG}" >/dev/null; then
  fail_with 11 "nihao_top_missing_target"
fi

REPLAY_BIN="${PROXY_DIR}/build/grpc-replay"
if [[ ! -x "${REPLAY_BIN}" ]]; then
  fail_with 13 "grpc_replay_missing"
fi

REPLAY_PORT=$(pick_available_port "${REPLAY_BASE_PORT}" || true)
if [[ -z "${REPLAY_PORT}" ]]; then
  fail_with 17 "replay_port_unavailable"
fi

log_line "[phase] replay start port=${REPLAY_PORT} input=${REPLAY_INPUT}"
WINEPREFIX="${WINE_PREFIX}" WINEDEBUG=-all RUST_LOG="${RUST_LOG:-}" IME_WINIMM_TRACE_TIMELINE="${IME_WINIMM_TRACE_TIMELINE:-0}" IME_GRPC_BIND="127.0.0.1:${REPLAY_PORT}" IME_POOL_MIN_IDLE="${REPLAY_POOL_MIN_IDLE}" IME_POOL_PREWARM=true IME_WORKER_BACKEND=win_imm IME_WINIMM_FORCE_REAL=1 IME_WINIMM_DLL="${DLL_PATH}" wine target/i686-pc-windows-gnu/debug/ime-grpc-host.exe > >(tee -a "${REPLAY_HOST_LOG}") 2>&1 &
REPLAY_HOST_PID=$!

ready_attempt=1
while [[ ${ready_attempt} -le ${REPLAY_READY_MAX_ATTEMPTS} ]]; do
  log_line "[phase] replay readiness attempt=${ready_attempt}/${REPLAY_READY_MAX_ATTEMPTS}"
  if ! kill -0 "${REPLAY_HOST_PID}" 2>/dev/null; then
    fail_with 20 "replay_host_exited_before_ready"
  fi

  if is_port_busy "${REPLAY_PORT}"; then
    log_line "[phase] replay readiness confirmed"
    break
  fi
  read -r -t 1 _ || true
  ready_attempt=$((ready_attempt + 1))
done

if [[ ${ready_attempt} -gt ${REPLAY_READY_MAX_ATTEMPTS} ]]; then
  fail_with 19 "replay_readiness_timeout"
fi

set +e
replay_rc=1
replay_attempt=1
while [[ ${replay_attempt} -le ${REPLAY_MAX_ATTEMPTS} ]]; do
  log_line "[phase] replay attempt=${replay_attempt}/${REPLAY_MAX_ATTEMPTS}"
  cd "${PROXY_DIR}"
  IME_GRPC_HOST=127.0.0.1 IME_GRPC_PORT="${REPLAY_PORT}" IME_GRPC_TIMEOUT_MS="${REPLAY_RPC_TIMEOUT_MS}" IME_REPLAY_INPUT="${REPLAY_INPUT}" IME_GRPC_TRACE_PER_KEY=1 ./build/grpc-replay 2>&1 | tee -a "${REPLAY_LOG}"
  replay_rc=$?
  cd "${ROOT_DIR}"

  if [[ ${replay_rc} -eq 0 ]]; then
    break
  fi

  read -r -t 1 _ || true
  replay_attempt=$((replay_attempt + 1))
done
set -e
if [[ ${replay_rc} -ne 0 ]]; then
  fail_with 14 "replay_rpc_failed"
fi

if ! grep -F "step 1 input='n'" "${REPLAY_LOG}" >/dev/null; then
  fail_with 15 "replay_chain_missing_step1"
fi
if ! grep -F "step 2 input='ni'" "${REPLAY_LOG}" >/dev/null; then
  fail_with 15 "replay_chain_missing_step2"
fi
if ! grep -F "step 3 input='nih'" "${REPLAY_LOG}" >/dev/null; then
  fail_with 15 "replay_chain_missing_step3"
fi
if ! grep -F "step 4 input='niha'" "${REPLAY_LOG}" >/dev/null; then
  fail_with 15 "replay_chain_missing_step4"
fi
if ! grep -F "step 5 input='nihao'" "${REPLAY_LOG}" >/dev/null; then
  fail_with 15 "replay_chain_missing_step5"
fi

cleanup
REPLAY_HOST_PID=""

if pgrep -f "target/i686-pc-windows-gnu/debug/ime-grpc-host.exe" >/dev/null; then
  fail_with 16 "stale_main_process_detected"
fi
if pgrep -f "ime-grpc-host.exe --worker-runtime" >/dev/null; then
  fail_with 16 "stale_worker_process_detected"
fi
if [[ -n "${REGRESSION_PORT}" ]] && is_port_busy "${REGRESSION_PORT}"; then
  fail_with 17 "regression_port_still_busy"
fi
if is_port_busy "${REPLAY_PORT}"; then
  fail_with 17 "replay_port_still_busy"
fi

log_line "[result] PASS"
log_line "[summary] acceptance_log=${ACCEPT_LOG}"
log_line "[summary] regression_log=${REGRESSION_LOG}"
log_line "[summary] replay_log=${REPLAY_LOG}"
log_line "[summary] nihao_log=${NIHAO_LOG}"
log_line "[summary] run_log=${RUN_LOG}"
