#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
HOST_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${SCRIPT_DIR}/../../../.." && pwd)"

PREFIX="${QQPY_WINEPREFIX:-${ROOT_DIR}/.wine32}"
DLL_PATH="${QQPY_DLL_PATH:-C:\\windows\\system32\\QQPinyin.ime}"
HOST_PORT="${QQPY_HOST_PORT:-22912}"
USE_XVFB="${QQPY_USE_XVFB:-1}"

TS="$(date +%Y%m%d_%H%M%S)"
CACHE_DIR="${ROOT_DIR}/.cache"
HOST_LOG="${CACHE_DIR}/qqpinyin_modes_host_${TS}.log"
CLIENT_LOG="${CACHE_DIR}/qqpinyin_modes_client_${TS}.log"

HOST_PID=""

cleanup() {
  if [[ -n "${HOST_PID}" ]] && kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" 2>/dev/null || true
    wait "${HOST_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

mkdir -p "${CACHE_DIR}"

echo "[qqpy-modes] host_dir=${HOST_DIR}"
echo "[qqpy-modes] prefix=${PREFIX}"
echo "[qqpy-modes] dll=${DLL_PATH}"
echo "[qqpy-modes] port=${HOST_PORT}"

cd "${HOST_DIR}"

if [[ ! -f "${HOST_DIR}/ime_host_skeleton.exe" ]]; then
  echo "[qqpy-modes] build host executable"
  i686-w64-mingw32-gcc ime_host_skeleton.c -o ime_host_skeleton.exe -lws2_32 -limm32
fi

HOST_CMD=(
  bash /opt/sogou/winabc/wine_run.sh
  "${HOST_DIR}/ime_host_skeleton.exe"
  --port "${HOST_PORT}"
  --dll "${DLL_PATH}"
)

echo "[qqpy-modes] start host"
if [[ "${USE_XVFB}" == "1" ]]; then
  xvfb-run -a env WINEPREFIX="${PREFIX}" WINEDEBUG=-all "${HOST_CMD[@]}" 2>&1 | tee "${HOST_LOG}" &
else
  env WINEPREFIX="${PREFIX}" WINEDEBUG=-all "${HOST_CMD[@]}" 2>&1 | tee "${HOST_LOG}" &
fi
HOST_PID=$!

echo "[qqpy-modes] run staged mode tests"
set +e
POC_HOST_PORT="${HOST_PORT}" python3 "${SCRIPT_DIR}/trace_client_qqpinyin_modes.py" 2>&1 | tee "${CLIENT_LOG}"
CLIENT_RC=${PIPESTATUS[0]}
set -e

echo "[qqpy-modes] client_rc=${CLIENT_RC}"
echo "[qqpy-modes] host_log=${HOST_LOG}"
echo "[qqpy-modes] client_log=${CLIENT_LOG}"

exit "${CLIENT_RC}"
