#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
POC_DIR="${ROOT_DIR}/poc"
CACHE_DIR="/opt/sogou/.cache"

GDB_PORT="${POC_GDB_PORT:-22701}"
HOST_PORT="${POC_HOST_PORT:-22702}"
DLL_PATH="${POC_DLL_PATH:-C:\\windows\\system32\\SogouPY.ime}"
CLIENT_SCRIPT="${POC_CLIENT_SCRIPT:-trace_client_force36c.py}"
GDB_SCRIPT="${POC_GDB_SCRIPT:-force36c_probe.gdb}"
USE_XVFB="${POC_USE_XVFB:-1}"
SHOW_WINDOW="${POC_SHOW_WINDOW:-0}"
WAIT_TIMEOUT_SEC="${POC_WAIT_TIMEOUT_SEC:-30}"
FILTER_X_BROKEN="${POC_FILTER_X_BROKEN:-1}"

TS="$(date +%Y%m%d_%H%M%S)"
HOST_LOG="${CACHE_DIR}/poc_force36c_host_${TS}.log"
GDB_LOG="${CACHE_DIR}/poc_force36c_gdb_${TS}.log"
CLIENT_LOG="${CACHE_DIR}/poc_force36c_client_${TS}.log"
GDB_CMD_FILE="${CACHE_DIR}/poc_force36c_cmd_${TS}.gdb"
GHOST_ADDR_HEX=""
HOST_IME_PTR_ADDR=""

HOST_PID=""
GDB_PID=""
CLIENT_RC=0

cleanup() {
  if [[ -n "${GDB_PID}" ]] && kill -0 "${GDB_PID}" 2>/dev/null; then
    kill "${GDB_PID}" || true
  fi
  if [[ -n "${HOST_PID}" ]] && kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" || true
  fi
}
trap cleanup EXIT

wait_pid_with_timeout() {
  local pid="$1"
  local label="$2"
  local timeout_sec="$3"
  local elapsed=0

  if [[ -z "${pid}" ]]; then
    return 0
  fi

  while kill -0 "${pid}" 2>/dev/null; do
    if (( elapsed >= timeout_sec )); then
      echo "[poc] WARN: timeout waiting ${label} (${timeout_sec}s), sending TERM pid=${pid}"
      kill "${pid}" 2>/dev/null || true
      sleep 1
      if kill -0 "${pid}" 2>/dev/null; then
        echo "[poc] WARN: ${label} still alive, sending KILL pid=${pid}"
        kill -9 "${pid}" 2>/dev/null || true
      fi
      break
    fi
    sleep 1
    elapsed=$((elapsed + 1))
  done

  wait "${pid}" 2>/dev/null || true
}

echo "[poc] build host (if needed)"
cd "${ROOT_DIR}"
i686-w64-mingw32-gcc ime_host_skeleton.c -o ime_host_skeleton.exe -lws2_32 -limm32 2>&1 | tee "${CACHE_DIR}/poc_force36c_build_${TS}.log"

GHOST_ADDR_HEX="$(i686-w64-mingw32-nm -n "${ROOT_DIR}/ime_host_skeleton.exe" | awk '/ _g_host$/{print $1; exit}')"
if [[ -z "${GHOST_ADDR_HEX}" ]]; then
  echo "[poc] ERROR: failed to resolve _g_host from nm" >&2
  exit 1
fi
HOST_IME_PTR_ADDR="0x$(printf '%x' $((16#${GHOST_ADDR_HEX} + 12)))"
echo "[poc] host_ime_ptr_addr=${HOST_IME_PTR_ADDR} (_g_host=0x${GHOST_ADDR_HEX})"

echo "[poc] materialize gdb script port=${GDB_PORT}"
cd "${POC_DIR}"
if [[ ! -f "${POC_DIR}/${GDB_SCRIPT}" ]]; then
  echo "[poc] ERROR: missing gdb script ${POC_DIR}/${GDB_SCRIPT}" >&2
  exit 1
fi

cat "${GDB_SCRIPT}" \
  | sed "s/__GDB_PORT__/${GDB_PORT}/g" \
  | sed "s/__HOST_IME_PTR_ADDR__/${HOST_IME_PTR_ADDR}/g" \
  | tee "${GDB_CMD_FILE}"

echo "[poc] start gdbserver host port=${HOST_PORT} xvfb=${USE_XVFB} show_window=${SHOW_WINDOW}"
cd "${ROOT_DIR}"
HOST_CMD=(
  bash /opt/sogou/winabc/wine_run.sh
  /usr/share/win32/gdbserver.exe "localhost:${GDB_PORT}"
  "${ROOT_DIR}/ime_host_skeleton.exe"
  --port "${HOST_PORT}"
  --dll "${DLL_PATH}"
)

if [[ "${SHOW_WINDOW}" == "1" ]]; then
  HOST_CMD+=(--show-window)
fi

if [[ "${USE_XVFB}" == "1" ]]; then
  if [[ "${FILTER_X_BROKEN}" == "1" ]]; then
    xvfb-run -a "${HOST_CMD[@]}" 2>&1 \
      | sed '/^X connection to :[0-9][0-9]* broken (explicit kill or server shutdown)\.$/d' \
      | tee "${HOST_LOG}" &
  else
    xvfb-run -a "${HOST_CMD[@]}" 2>&1 | tee "${HOST_LOG}" &
  fi
else
  if [[ -z "${DISPLAY:-}" ]]; then
    echo "[poc] WARN: DISPLAY is empty while POC_USE_XVFB=0; GUI may not be visible"
  fi
  if [[ "${FILTER_X_BROKEN}" == "1" ]]; then
    "${HOST_CMD[@]}" 2>&1 \
      | sed '/^X connection to :[0-9][0-9]* broken (explicit kill or server shutdown)\.$/d' \
      | tee "${HOST_LOG}" &
  else
    "${HOST_CMD[@]}" 2>&1 | tee "${HOST_LOG}" &
  fi
fi
HOST_PID=$!

echo "[poc] start gdb probe"
if [[ "${FILTER_X_BROKEN}" == "1" ]]; then
  i686-w64-mingw32-gdb "${ROOT_DIR}/ime_host_skeleton.exe" -q -batch -x "${GDB_CMD_FILE}" 2>&1 \
    | sed '/^X connection to :[0-9][0-9]* broken (explicit kill or server shutdown)\.$/d' \
    | tee "${GDB_LOG}" &
else
  i686-w64-mingw32-gdb "${ROOT_DIR}/ime_host_skeleton.exe" -q -batch -x "${GDB_CMD_FILE}" 2>&1 | tee "${GDB_LOG}" &
fi
GDB_PID=$!

if [[ ! -f "${POC_DIR}/${CLIENT_SCRIPT}" ]]; then
  echo "[poc] ERROR: missing client script ${POC_DIR}/${CLIENT_SCRIPT}" >&2
  exit 1
fi

echo "[poc] run client script=${CLIENT_SCRIPT}"
cd "${POC_DIR}"
set +e
POC_HOST_PORT="${HOST_PORT}" python3 "${CLIENT_SCRIPT}" 2>&1 | tee "${CLIENT_LOG}"
CLIENT_RC=${PIPESTATUS[0]}
set -e
echo "[poc] client_rc=${CLIENT_RC}"

echo "[poc] wait gdb/host"
wait_pid_with_timeout "${GDB_PID}" "gdb" "${WAIT_TIMEOUT_SEC}"
wait_pid_with_timeout "${HOST_PID}" "host" "${WAIT_TIMEOUT_SEC}"

echo "[poc] done"
echo "[poc] host_log=${HOST_LOG}"
echo "[poc] gdb_log=${GDB_LOG}"
echo "[poc] client_log=${CLIENT_LOG}"
echo "[poc] gdb_cmd=${GDB_CMD_FILE}"
exit "${CLIENT_RC}"
