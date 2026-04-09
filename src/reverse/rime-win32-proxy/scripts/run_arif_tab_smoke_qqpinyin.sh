#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROXY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${PROXY_DIR}/../../.." && pwd)"
HOST_DIR="${ROOT_DIR}/src/reverse/host"
ARIF_DIR="${ROOT_DIR}/arif"

CACHE_DIR="${ROOT_DIR}/.cache"
USER_DIR="${QQPY_RIME_USER_DIR:-${CACHE_DIR}/rime-win32-proxy-user}"
SHARED_DIR="${ARIF_RIME_SHARED_DATA_DIR:-/usr/share/rime-data}"
STAGING_DIR="${USER_DIR}/build"
WINEPREFIX_DIR="${QQPY_WINEPREFIX:-${ROOT_DIR}/.wine32}"
HOST_PORT="${QQPY_HOST_PORT:-22912}"
DLL_PATH="${QQPY_DLL_PATH:-C:\\windows\\system32\\QQPinyin.ime}"
USE_XVFB="${QQPY_USE_XVFB:-1}"
INPUT_TEXT="${QQPY_INPUT_TEXT:-ni}"
INPUTS_CSV="${QQPY_INPUTS_CSV:-}"

ARIFY_BIN="${ARIF_DIR}/build/src/arify"
LIBARIFY_SO="${ARIF_DIR}/build/src/.libs/libarify.so"
LIBARIF_RIME_SO="${ARIF_DIR}/build/src/.libs/libarif_rime.so"
PROXY_SO="${PROXY_DIR}/build/librime-win32-proxy.so"
EXPECT_SCRIPT="${SCRIPT_DIR}/arif_tab_smoke.expect"

TS="$(date +%Y%m%d_%H%M%S)"
HOST_LOG="${CACHE_DIR}/qqpy_arif_tab_host_${TS}.log"
DEPLOY_LOG="${CACHE_DIR}/qqpy_arif_tab_deploy_${TS}.log"
EXPECT_LOG="${CACHE_DIR}/qqpy_arif_tab_expect_${TS}.log"
ARIFY_LOG="${CACHE_DIR}/qqpy_arif_tab_arify_${TS}.log"
QUIT_LOG="${CACHE_DIR}/qqpy_arif_tab_quit_${TS}.log"
RIME_LOG_SCAN="${CACHE_DIR}/qqpy_arif_tab_rime_log_scan_${TS}.log"
RIME_LOG_DIR="${CACHE_DIR}/qqpy_arif_tab_rime_${TS}"
SCHEMA_PATCH_FILE="${USER_DIR}/win32_proxy.custom.yaml"

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

if ! python3 -c "import socket,sys; p=int(sys.argv[1]); s=socket.socket();\
s.bind(('127.0.0.1', p)); s.close()" "${HOST_PORT}" >/dev/null 2>&1; then
  original_port="${HOST_PORT}"
  found_port=""
  for candidate in $(seq "${HOST_PORT}" "$((HOST_PORT + 120))"); do
    if python3 -c "import socket,sys; p=int(sys.argv[1]); s=socket.socket(); s.bind(('127.0.0.1', p)); s.close()" "${candidate}" >/dev/null 2>&1; then
      found_port="${candidate}"
      break
    fi
  done
  if [[ -z "${found_port}" ]]; then
    echo "[qqpy-arif-tab] no free port near ${original_port}" >&2
    exit 2
  fi
  HOST_PORT="${found_port}"
  echo "[qqpy-arif-tab] port ${original_port} busy, switched to ${HOST_PORT}"
fi

echo "[qqpy-arif-tab] host_port=${HOST_PORT}"

for f in "${ARIFY_BIN}" "${LIBARIFY_SO}" "${LIBARIF_RIME_SO}" "${PROXY_SO}" "${EXPECT_SCRIPT}" "${HOST_DIR}/ime_host_skeleton.exe"; do
  if [[ ! -f "${f}" ]]; then
    echo "[qqpy-arif-tab] missing required file: ${f}" >&2
    exit 2
  fi
done

cp -f "${PROXY_DIR}/schema/win32_proxy.schema.yaml" "${USER_DIR}/win32_proxy.schema.yaml"
printf 'patch:\n  win32_proxy/host: "127.0.0.1"\n  win32_proxy/port: %s\n  win32_proxy/command: "TEXTU"\n  win32_proxy/timeout_ms: 2500\n' "${HOST_PORT}" > "${SCHEMA_PATCH_FILE}"

{
  cd "${USER_DIR}"
  rime_deployer --add-schema win32_proxy
  rime_deployer --set-active-schema win32_proxy
  rime_deployer --build "${USER_DIR}" "${SHARED_DIR}" "${STAGING_DIR}"
} 2>&1 | tee "${DEPLOY_LOG}"

echo "[qqpy-arif-tab] starting host"
HOST_CMD=(bash /opt/sogou/winabc/wine_run.sh "${HOST_DIR}/ime_host_skeleton.exe" --port "${HOST_PORT}" --dll "${DLL_PATH}")
if [[ "${USE_XVFB}" == "1" ]]; then
  xvfb-run -a env WINEPREFIX="${WINEPREFIX_DIR}" WINEDEBUG=-all "${HOST_CMD[@]}" 2>&1 | tee "${HOST_LOG}" &
else
  env WINEPREFIX="${WINEPREFIX_DIR}" WINEDEBUG=-all "${HOST_CMD[@]}" 2>&1 | tee "${HOST_LOG}" &
fi
HOST_PID=$!

host_ready=0
for _ in $(seq 1 300); do
  if python3 -c $'import socket,sys\np=int(sys.argv[1])\ntry:\n s=socket.create_connection((\'127.0.0.1\', p), timeout=0.3)\n s.settimeout(0.8)\n f=s.makefile(\'rwb\', buffering=0)\n hello=f.readline().decode(\'utf-8\',\'replace\').strip()\n if not hello.startswith(\'HELLO \'):\n  raise RuntimeError(\'no_hello\')\n f.write(b\'PING\\n\')\n pong=f.readline().decode(\'utf-8\',\'replace\').strip()\n if pong != \'PONG\':\n  raise RuntimeError(\'no_pong\')\n f.write(b\'STATUS\\n\')\n status=f.readline().decode(\'utf-8\',\'replace\').strip()\n if not status.startswith(\'STATUS \'):\n  raise RuntimeError(\'bad_status\')\n if \'select=\' in status and \'select=1\' not in status:\n  raise RuntimeError(\'select_not_ready\')\n s.close()\n sys.exit(0)\nexcept Exception:\n sys.exit(1)\n' "${HOST_PORT}" >/dev/null 2>&1; then
    echo "[qqpy-arif-tab] host_reachable=1 host_protocol_ready=1" | tee -a "${HOST_LOG}"
    host_ready=1
    break
  fi
  sleep 0.1
done

if [[ ${host_ready} -ne 1 ]]; then
  echo "[qqpy-arif-tab] host not ready" >&2
  exit 1
fi

echo "[qqpy-arif-tab] running expect tab smoke"
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
  "${INPUTS_CSV}" 2>&1 | tee "${EXPECT_LOG}"

set +e
rg "win32_proxy translator initialized|bad reply|connect failed|no greeting" "${RIME_LOG_DIR}" 2>&1 | tee "${RIME_LOG_SCAN}"
RG_RC=${PIPESTATUS[0]}
set -e
if [[ ${RG_RC} -ne 0 ]]; then
  echo "[qqpy-arif-tab] warning: no matching proxy log lines in ${RIME_LOG_DIR}"
fi

python3 -c "import socket,sys; p=int(sys.argv[1]); s=socket.create_connection(('127.0.0.1',p),timeout=2); f=s.makefile('rwb', buffering=0); print(f.readline().decode('utf-8','replace').strip()); f.write(b'QUIT\\n'); print(f.readline().decode('utf-8','replace').strip()); s.close()" "${HOST_PORT}" 2>&1 | tee "${QUIT_LOG}"

wait "${HOST_PID}" || true
HOST_PID=""

echo "[qqpy-arif-tab] done"
echo "[qqpy-arif-tab] deploy_log=${DEPLOY_LOG}"
echo "[qqpy-arif-tab] host_log=${HOST_LOG}"
echo "[qqpy-arif-tab] expect_log=${EXPECT_LOG}"
echo "[qqpy-arif-tab] arify_log=${ARIFY_LOG}"
echo "[qqpy-arif-tab] rime_log_dir=${RIME_LOG_DIR}"
echo "[qqpy-arif-tab] rime_log_scan=${RIME_LOG_SCAN}"
