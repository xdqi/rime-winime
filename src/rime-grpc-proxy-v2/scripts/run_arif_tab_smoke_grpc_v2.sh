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
STAGING_DIR="${USER_DIR}/build"

ARIFY_BIN="${ARIF_DIR}/build/src/arify"
LIBARIFY_SO="${ARIF_DIR}/build/src/.libs/libarify.so"
LIBARIF_RIME_SO="${ARIF_DIR}/build/src/.libs/libarif_rime.so"
PROXY_SO="${PROXY_DIR}/build/librime-grpc-proxy-v2.so"
EXPECT_SCRIPT="${SCRIPT_DIR}/arif_tab_smoke.expect"

TS="$(date +%Y%m%d_%H%M%S)"
HOST_LOG="${CACHE_DIR}/grpc_v2_host_${TS}.log"
DEPLOY_LOG="${CACHE_DIR}/grpc_v2_deploy_${TS}.log"
EXPECT_LOG="${CACHE_DIR}/grpc_v2_expect_${TS}.log"
ARIFY_LOG="${CACHE_DIR}/grpc_v2_arify_${TS}.log"
RIME_LOG_DIR="${CACHE_DIR}/grpc_v2_rime_${TS}"

INPUT_TEXT="ni"

HOST_PID=""
cleanup() {
  if [[ -n "${HOST_PID}" ]] && kill -0 "${HOST_PID}" 2>/dev/null; then
    kill "${HOST_PID}" 2>/dev/null || true
    wait "${HOST_PID}" 2>/dev/null || true
  fi
}
trap cleanup EXIT

mkdir -p "${CACHE_DIR}" "${USER_DIR}" "${RIME_LOG_DIR}"

for f in "${ARIFY_BIN}" "${LIBARIFY_SO}" "${LIBARIF_RIME_SO}" "${PROXY_SO}" "${EXPECT_SCRIPT}"; do
  if [[ ! -f "${f}" ]]; then
    echo "[grpc-v2-smoke] missing required file: ${f}" >&2
    exit 2
  fi
done

# Copy schema into RIME user dir and build it
cp -f "${PROXY_DIR}/grpc_proxy.schema.yaml" "${USER_DIR}/grpc_proxy.schema.yaml"

{
  cd "${USER_DIR}"
  # Normally rime_deployer installs schema
  # We just set it up manually
  rime_deployer --add-schema grpc_proxy
  rime_deployer --set-active-schema grpc_proxy
  rime_deployer --build "${USER_DIR}" "${SHARED_DIR}" "${STAGING_DIR}"
} 2>&1 | tee "${DEPLOY_LOG}"

# Start Host
echo "[grpc-v2-smoke] starting host via Wine"
cd "${HOST_DIR}"
env RUST_LOG=info WINEPREFIX=/opt/sogou/.wine32 wine "${HOST_DIR}/target/i686-pc-windows-gnu/debug/ime-grpc-host-v2.exe" > "${HOST_LOG}" 2>&1 &
HOST_PID=$!
echo "[grpc-v2-smoke] Waiting for host to bind port 50051..."
for i in {1..30}; do
  if ss -tln | grep -q ':50051 '; then
    echo "Host is ready!"
    break
  fi
  sleep 1
done

echo "[grpc-v2-smoke] running expect tab smoke"
EXPECT_ENV="LD_PRELOAD=${PROXY_SO} \
  ARIFY_ENGINES=${LIBARIF_RIME_SO}:arif_rime_engine \
  ARIFY_FRONTEND=readline \
  ARIFY_PAGE_SIZE=9 \
  ARIFY_LOG_FILE=${ARIFY_LOG} \
  ARIFY_RL_NO_AUTO_UNSETENV=1 \
  ARIF_RIME_MODULES=default,grpc_proxy_v2 \
  ARIF_RIME_USER_DATA_DIR=${USER_DIR} \
  ARIF_RIME_SHARED_DATA_DIR=${SHARED_DIR} \
  ARIF_RIME_LOG_DIR=${RIME_LOG_DIR} \
  ARIF_RIME_LOG_LEVEL=INFO"

env LD_PRELOAD="${PROXY_SO}" \
    ARIFY_ENGINES="${LIBARIF_RIME_SO}:arif_rime_engine" \
    ARIFY_FRONTEND="readline" \
    ARIFY_PAGE_SIZE="9" \
    ARIFY_LOG_FILE="${ARIFY_LOG}" \
    ARIFY_RL_NO_AUTO_UNSETENV="1" \
    ARIF_RIME_MODULES="default,grpc_proxy_v2" \
    ARIF_RIME_USER_DATA_DIR="${USER_DIR}" \
    ARIF_RIME_SHARED_DATA_DIR="${SHARED_DIR}" \
    ARIF_RIME_LOG_DIR="${RIME_LOG_DIR}" \
    ARIF_RIME_LOG_LEVEL="INFO" \
    "${EXPECT_SCRIPT}" \
      "${ARIFY_BIN}" \
      "${LIBARIFY_SO}" \
      "${LIBARIF_RIME_SO}" \
      "${PROXY_SO}" \
      "${USER_DIR}" \
      "${SHARED_DIR}" \
      "${ARIFY_LOG}" \
      "${RIME_LOG_DIR}" \
      "${INPUT_TEXT}" \
      "" 2>&1 | tee "${EXPECT_LOG}"


echo "[grpc-v2-smoke] done"
echo "[grpc-v2-smoke] deploy_log=${DEPLOY_LOG}"
echo "[grpc-v2-smoke] host_log=${HOST_LOG}"
echo "[grpc-v2-smoke] expect_log=${EXPECT_LOG}"
echo "[grpc-v2-smoke] rime_log_dir=${RIME_LOG_DIR}"
