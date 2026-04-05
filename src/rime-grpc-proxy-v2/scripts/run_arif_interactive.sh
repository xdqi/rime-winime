#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROXY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${PROXY_DIR}/../.." && pwd)"

HOST_DIR="${ROOT_DIR}/src/ime-grpc-host-v2"
ARIF_DIR="${ROOT_DIR}/arif"

CACHE_DIR="${ROOT_DIR}/.cache"
USER_DIR="${CACHE_DIR}/rime-grpc-v2-user"
SHARED_DIR="/usr/share/rime-data"

ARIFY_BIN="${ARIF_DIR}/build/src/arify"
LIBARIFY_SO="${ARIF_DIR}/build/src/.libs/libarify.so"
LIBARIF_RIME_SO="${ARIF_DIR}/build/src/.libs/libarif_rime.so"
PROXY_SO="${PROXY_DIR}/build/librime-grpc-proxy-v2.so"

for f in "${ARIFY_BIN}" "${LIBARIFY_SO}" "${LIBARIF_RIME_SO}" "${PROXY_SO}"; do
  if [[ ! -f "${f}" ]]; then
    echo "[grpc-v2-interactive] missing required file: ${f}" >&2
    exit 2
  fi
done

echo "[grpc-v2-interactive] Starting Host in background via Wine..."
cd "${HOST_DIR}"
env RUST_LOG=info GLOG_log_dir="${CACHE_DIR}" WINEPREFIX=/opt/sogou/.wine32 wine "${HOST_DIR}/target/i686-pc-windows-gnu/debug/ime-grpc-host-v2.exe" > "${CACHE_DIR}/interactive_host.log" 2>&1 &
HOST_PID=$!

cleanup() {
  echo "[grpc-v2-interactive] Cleaning up host (${HOST_PID})..."
  if kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

echo "[grpc-v2-interactive] Waiting for host to bind port 50051..."
for i in {1..30}; do
  if ss -tln | grep -q ':50051 '; then
    echo "Host is ready!"
    break
  fi
  sleep 0.5
done
echo "==================================================================="
echo "[grpc-v2-interactive] Entering interactive shell."
echo "   - Press Ctrl-x then Ctrl-a to toggle the IME."
echo "   - Type to see if Rime works via the gRPC pipe!"
echo "   - Type 'exit' or Ctrl-D to quit."
echo "==================================================================="

env LD_PRELOAD="${PROXY_SO}" \
    GLOG_log_dir="${CACHE_DIR}" \
    ARIFY_ENGINES="${LIBARIF_RIME_SO}:arif_rime_engine" \
    ARIFY_FRONTEND="readline" \
    ARIFY_PAGE_SIZE="9" \
    ARIFY_RL_NO_AUTO_UNSETENV="1" \
    ARIF_RIME_MODULES="default,grpc_proxy_v2" \
    ARIF_RIME_USER_DATA_DIR="${USER_DIR}" \
    ARIF_RIME_SHARED_DATA_DIR="${SHARED_DIR}" \
    ARIF_RIME_LOG_LEVEL="INFO" \
    "${ARIFY_BIN}" -p "${LIBARIFY_SO}" -f readline -- bash --rcfile "${CACHE_DIR}/interactive_bashrc"
