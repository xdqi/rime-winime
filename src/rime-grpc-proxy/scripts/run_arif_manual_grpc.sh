#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROXY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${PROXY_DIR}/../.." && pwd)"
HOST_DIR="${ROOT_DIR}/src/ime-grpc-host"
ARIF_DIR="${ROOT_DIR}/arif"

CACHE_DIR="${ROOT_DIR}/.cache"
USER_DIR="${QQPY_RIME_USER_DIR:-${CACHE_DIR}/rime-grpc-proxy-user}"
SHARED_DIR="${ARIF_RIME_SHARED_DATA_DIR:-/usr/share/rime-data}"
STAGING_DIR="${USER_DIR}/build"
WINEPREFIX_DIR="${QQPY_WINEPREFIX:-${ROOT_DIR}/.wine32}"
HOST_PORT="${QQPY_HOST_PORT:-50096}"
DLL_PATH="${QQPY_DLL_PATH:-C:\\windows\\system32\\QQPinyin.ime}"
NO_HOST="${QQPY_NO_HOST:-0}"

ARIFY_BIN="${ARIF_DIR}/build/src/arify"
LIBARIFY_SO="${ARIF_DIR}/build/src/.libs/libarify.so"
LIBARIF_RIME_SO="${ARIF_DIR}/build/src/.libs/libarif_rime.so"
PROXY_SO="${PROXY_DIR}/build/librime-grpc-proxy.so"

TS="$(date +%Y%m%d_%H%M%S)"
HOST_LOG="${CACHE_DIR}/qqpy_arif_manual_host_${TS}.log"
DEPLOY_LOG="${CACHE_DIR}/qqpy_arif_manual_deploy_${TS}.log"
ARIFY_LOG="${CACHE_DIR}/qqpy_arif_manual_arify_${TS}.log"
RC_FILE="${USER_DIR}/arif_manual_${TS}.bashrc"
SCHEMA_PATCH_FILE="${USER_DIR}/grpc_proxy.custom.yaml"

HOST_PID=""
HOST_STARTED_BY_SCRIPT=0

cleanup() {
  if [[ "${HOST_STARTED_BY_SCRIPT}" == "1" ]] && [[ -n "${HOST_PID}" ]] && kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" 2>/dev/null || true
    wait "${HOST_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

mkdir -p "${CACHE_DIR}" "${USER_DIR}" "${USER_DIR}/log"

echo "[qqpy-arif-manual] root=${ROOT_DIR}"
echo "[qqpy-arif-manual] user_dir=${USER_DIR}"
echo "[qqpy-arif-manual] shared_dir=${SHARED_DIR}"
echo "[qqpy-arif-manual] dll=${DLL_PATH}"

for f in "${ARIFY_BIN}" "${LIBARIFY_SO}" "${LIBARIF_RIME_SO}" "${PROXY_SO}"; do
  if [[ ! -f "${f}" ]]; then
    echo "[qqpy-arif-manual] missing required file: ${f}" >&2
    exit 2
  fi
done

if [[ "${NO_HOST}" != "1" ]]; then
  if [[ ! -f "${HOST_DIR}/target/i686-pc-windows-gnu/debug/ime-grpc-host.exe" ]]; then
    echo "[qqpy-arif-manual] missing required file: ${HOST_DIR}/target/i686-pc-windows-gnu/debug/ime-grpc-host.exe" >&2
    exit 2
  fi
fi

cp -f "${PROXY_DIR}/schema/grpc_proxy.schema.yaml" "${USER_DIR}/grpc_proxy.schema.yaml"
printf 'patch:\n  grpc_proxy/host: "127.0.0.1"\n  grpc_proxy/port: %s\n  grpc_proxy/timeout_ms: 5000\n' "${HOST_PORT}" > "${SCHEMA_PATCH_FILE}"

{
  cd "${USER_DIR}"
  rime_deployer --add-schema grpc_proxy
  rime_deployer --set-active-schema grpc_proxy
  rime_deployer --build "${USER_DIR}" "${SHARED_DIR}" "${STAGING_DIR}"
} 2>&1 | tee "${DEPLOY_LOG}"

if [[ "${NO_HOST}" == "1" ]]; then
  echo "[qqpy-arif-manual] using existing host on 127.0.0.1:${HOST_PORT}"
else
  echo "[qqpy-arif-manual] starting host: ${DLL_PATH}"
  env WINEPREFIX="${WINEPREFIX_DIR}" \
    WINEDEBUG=-all \
    IME_GRPC_BIND="127.0.0.1:${HOST_PORT}" \
    IME_WORKER_BACKEND=win_imm \
    IME_WINIMM_FORCE_REAL=1 \
    IME_WINIMM_DLL="${DLL_PATH}" \
    IME_POOL_MIN_IDLE="${IME_POOL_MIN_IDLE:-3}" \
    RUST_LOG=info \
    wine "${HOST_DIR}/target/i686-pc-windows-gnu/debug/ime-grpc-host.exe" > >(tee -a "${HOST_LOG}") 2>&1 &
  HOST_PID=$!
  HOST_STARTED_BY_SCRIPT=1

  host_ready=0
  for _ in $(seq 1 120); do
    if ss -ltn | awk -v port=":${HOST_PORT}" '$4 ~ port"$" { found=1 } END { exit(found ? 0 : 1) }'; then
      echo "[qqpy-arif-manual] host reachable"
      host_ready=1
      break
    fi
    sleep 0.5
  done

  if [[ "${host_ready}" == "0" ]]; then
    echo "[qqpy-arif-manual] host failed to start or bind" >&2
    exit 1
  fi
fi

cat << 'EOF' > "${RC_FILE}"
bind '"\C-x\C-a": arify-toggle'
echo "Welcome to ARIF Manual Tester (gRPC proxy mode)"
echo "Press Ctrl-X Ctrl-A to enable ARIF, then start typing to see QQ Pinyin candidates."
echo "Press Ctrl-C or type 'exit' to quit."
EOF

echo "[qqpy-arif-manual] dropping into shell..."
env LD_PRELOAD="${PROXY_SO}" \
  ARIFY_ENGINES="${LIBARIF_RIME_SO}:arif_rime_engine" \
  ARIFY_FRONTEND="readline" \
  ARIFY_PAGE_SIZE=9 \
  ARIFY_LOG_FILE="${ARIFY_LOG}" \
  ARIFY_RL_NO_AUTO_UNSETENV=1 \
  ARIF_RIME_MODULES="default,grpc_proxy" \
  ARIF_RIME_USER_DATA_DIR="${USER_DIR}" \
  ARIF_RIME_SHARED_DATA_DIR="${SHARED_DIR}" \
  ARIF_RIME_LOG_DIR="${USER_DIR}/log" \
  ARIF_RIME_LOG_LEVEL="INFO" \
  "${ARIFY_BIN}" -p "${LIBARIFY_SO}" -f readline -- bash --rcfile "${RC_FILE}" -i

echo "[qqpy-arif-manual] shell exited."