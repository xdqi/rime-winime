#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
cd "${ROOT_DIR}"

mkdir -p .cache

STAMP=$(date +%Y%m%d_%H%M%S_%N)
LOG_FILE=".cache/qq-debug-stop-force-disable-${STAMP}.log"

HOST=${IME_DEBUG_STOP_HOST:-127.0.0.1}
PORT=${IME_DEBUG_STOP_PORT:-59999}
TIMEOUT_MS=${IME_DEBUG_STOP_TIMEOUT_MS:-120}
INPUT=${IME_DEBUG_STOP_INPUT:-nihao}

if [[ ! -x "./build/grpc-replay" ]]; then
  echo "[FAIL] missing ./build/grpc-replay; build rime-grpc-proxy first"
  exit 2
fi

echo "[phase] trigger debug-stop with unreachable endpoint ${HOST}:${PORT}" | tee -a "${LOG_FILE}"

set +e
IME_GRPC_HOST="${HOST}" \
IME_GRPC_PORT="${PORT}" \
IME_GRPC_TIMEOUT_MS="${TIMEOUT_MS}" \
IME_REPLAY_INPUT="${INPUT}" \
IME_GRPC_DEBUG_STOP_MODE=1 \
./build/grpc-replay 2>&1 | tee -a "${LOG_FILE}"
RC=$?
set -e

if [[ ${RC} -eq 0 ]]; then
  echo "[FAIL] expected non-zero exit when debug_stop_mode is enabled" | tee -a "${LOG_FILE}"
  echo "[summary] log=${LOG_FILE}" | tee -a "${LOG_FILE}"
  exit 3
fi

if ! grep -F "debug_stop_mode triggered" "${LOG_FILE}" >/dev/null; then
  echo "[FAIL] missing debug_stop_mode visibility log marker" | tee -a "${LOG_FILE}"
  echo "[summary] log=${LOG_FILE}" | tee -a "${LOG_FILE}"
  exit 4
fi

echo "[PASS] debug-stop force-disable behavior is visible" | tee -a "${LOG_FILE}"
echo "[summary] exit_code=${RC}" | tee -a "${LOG_FILE}"
echo "[summary] log=${LOG_FILE}" | tee -a "${LOG_FILE}"
