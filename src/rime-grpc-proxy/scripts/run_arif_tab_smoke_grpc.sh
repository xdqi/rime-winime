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
INPUT_TEXT="${QQPY_INPUT_TEXT:-ni}"
INPUTS_CSV="${QQPY_INPUTS_CSV:-}"

ARIFY_BIN="${ARIF_DIR}/build/src/arify"
LIBARIFY_SO="${ARIF_DIR}/build/src/.libs/libarify.so"
LIBARIF_RIME_SO="${ARIF_DIR}/build/src/.libs/libarif_rime.so"
PROXY_SO="${PROXY_DIR}/build/librime-grpc-proxy.so"
EXPECT_SCRIPT="${SCRIPT_DIR}/arif_tab_smoke.expect"

TS="$(date +%Y%m%d_%H%M%S)"
HOST_LOG="${CACHE_DIR}/qqpy_arif_tab_host_${TS}.log"
DEPLOY_LOG="${CACHE_DIR}/qqpy_arif_tab_deploy_${TS}.log"
EXPECT_LOG="${CACHE_DIR}/qqpy_arif_tab_expect_${TS}.log"
ARIFY_LOG="${CACHE_DIR}/qqpy_arif_tab_arify_${TS}.log"
RIME_LOG_DIR="${CACHE_DIR}/qqpy_arif_tab_rime_${TS}"
SCHEMA_PATCH_FILE="${USER_DIR}/grpc_proxy.custom.yaml"

HOST_PID=""

cleanup() {
  if [[ -n "${HOST_PID}" ]] && kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" 2>/dev/null || true
    wait "${HOST_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

mkdir -p "${CACHE_DIR}" "${USER_DIR}" "${RIME_LOG_DIR}"

echo "[qqpy-arif-tab] root=${ROOT_DIR}"
echo "[qqpy-arif-tab] user_dir=${USER_DIR}"
echo "[qqpy-arif-tab] shared_dir=${SHARED_DIR}"
echo "[qqpy-arif-tab] dll=${DLL_PATH}"
echo "[qqpy-arif-tab] input_text=${INPUT_TEXT}"
if [[ -n "${INPUTS_CSV}" ]]; then
  echo "[qqpy-arif-tab] inputs_csv=${INPUTS_CSV}"
fi

for f in "${ARIFY_BIN}" "${LIBARIFY_SO}" "${LIBARIF_RIME_SO}" "${PROXY_SO}" "${EXPECT_SCRIPT}" "${HOST_DIR}/target/i686-pc-windows-gnu/debug/ime-grpc-host.exe"; do
  if [[ ! -f "${f}" ]]; then
    echo "[qqpy-arif-tab] missing required file: ${f}" >&2
    exit 2
  fi
done

cp -f "${PROXY_DIR}/schema/grpc_proxy.schema.yaml" "${USER_DIR}/grpc_proxy.schema.yaml"
printf 'patch:\n  grpc_proxy/host: "127.0.0.1"\n  grpc_proxy/port: %s\n  grpc_proxy/timeout_ms: 5000\n' "${HOST_PORT}" > "${SCHEMA_PATCH_FILE}"

{
  cd "${USER_DIR}"
  rime_deployer --add-schema grpc_proxy
  rime_deployer --set-active-schema grpc_proxy
  rime_deployer --build "${USER_DIR}" "${SHARED_DIR}" "${STAGING_DIR}"
} 2>&1 | tee "${DEPLOY_LOG}"

echo "[qqpy-arif-tab] starting host: ${DLL_PATH}"
env WINEPREFIX="${WINEPREFIX_DIR}" \
  WINEDEBUG=-all \
  IME_GRPC_BIND="127.0.0.1:${HOST_PORT}" \
  IME_WORKER_BACKEND=win_imm \
  IME_WINIMM_FORCE_REAL=1 \
  IME_WINIMM_DLL="${DLL_PATH}" \
  IME_POOL_MIN_IDLE="${IME_POOL_MIN_IDLE:-1}" \
  RUST_LOG=info \
  wine "${HOST_DIR}/target/i686-pc-windows-gnu/debug/ime-grpc-host.exe" > >(tee -a "${HOST_LOG}") 2>&1 &
HOST_PID=$!

# Wait for host
host_ready=0
for _ in $(seq 1 120); do
  if ss -ltn | awk -v port=":${HOST_PORT}" '$4 ~ port"$" { found=1 } END { exit(found ? 0 : 1) }'; then
    echo "[qqpy-arif-tab] host reachable"
    host_ready=1
    break
  fi
  sleep 0.5
done

if [[ "${host_ready}" == "0" ]]; then
  echo "[qqpy-arif-tab] host failed to start or bind" >&2
  exit 1
fi

echo "[qqpy-arif-tab] running expect"
chmod +x "${EXPECT_SCRIPT}"
stdbuf -oL expect "${EXPECT_SCRIPT}" "${ARIFY_BIN}" "${LIBARIFY_SO}" "${LIBARIF_RIME_SO}" "${PROXY_SO}" "${USER_DIR}" "${SHARED_DIR}" "${ARIFY_LOG}" "${RIME_LOG_DIR}" "${INPUT_TEXT}" "${INPUTS_CSV}" | tee "${EXPECT_LOG}"

if grep -q "warning: no visible output after TAB" "${EXPECT_LOG}"; then
  echo "[qqpy-arif-tab] expect reported no candidate output, failing" >&2
  exit 1
fi

echo "[qqpy-arif-tab] success!"
