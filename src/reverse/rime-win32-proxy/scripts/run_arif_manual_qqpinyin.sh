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
NO_HOST="${QQPY_NO_HOST:-0}"

ARIFY_BIN="${ARIF_DIR}/build/src/arify"
LIBARIFY_SO="${ARIF_DIR}/build/src/.libs/libarify.so"
LIBARIF_RIME_SO="${ARIF_DIR}/build/src/.libs/libarif_rime.so"
PROXY_SO="${PROXY_DIR}/build/librime-win32-proxy.so"

TS="$(date +%Y%m%d_%H%M%S)"
HOST_LOG="${CACHE_DIR}/qqpy_arif_manual_host_${TS}.log"
DEPLOY_LOG="${CACHE_DIR}/qqpy_arif_manual_deploy_${TS}.log"
ARIFY_LOG="${CACHE_DIR}/qqpy_arif_manual_arify_${TS}.log"
RC_FILE="${USER_DIR}/arif_manual_${TS}.bashrc"
SCHEMA_PATCH_FILE="${USER_DIR}/win32_proxy.custom.yaml"

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
  if [[ ! -f "${HOST_DIR}/ime_host_skeleton.exe" ]]; then
    echo "[qqpy-arif-manual] missing required file: ${HOST_DIR}/ime_host_skeleton.exe" >&2
    exit 2
  fi

  if ! python3 -c "import socket,sys; p=int(sys.argv[1]); s=socket.socket(); s.bind(('127.0.0.1', p)); s.close()" "${HOST_PORT}" >/dev/null 2>&1; then
    original_port="${HOST_PORT}"
    found_port=""
    for candidate in $(seq "${HOST_PORT}" "$((HOST_PORT + 120))"); do
      if python3 -c "import socket,sys; p=int(sys.argv[1]); s=socket.socket(); s.bind(('127.0.0.1', p)); s.close()" "${candidate}" >/dev/null 2>&1; then
        found_port="${candidate}"
        break
      fi
    done
    if [[ -z "${found_port}" ]]; then
      echo "[qqpy-arif-manual] no free port near ${original_port}" >&2
      exit 2
    fi
    HOST_PORT="${found_port}"
    echo "[qqpy-arif-manual] port ${original_port} busy, switched to ${HOST_PORT}"
  fi
fi

echo "[qqpy-arif-manual] host_port=${HOST_PORT}"

cp -f "${PROXY_DIR}/schema/win32_proxy.schema.yaml" "${USER_DIR}/win32_proxy.schema.yaml"
printf 'patch:\n  win32_proxy/host: "127.0.0.1"\n  win32_proxy/port: %s\n  win32_proxy/command: "TEXTU"\n  win32_proxy/timeout_ms: 2500\n' "${HOST_PORT}" > "${SCHEMA_PATCH_FILE}"

{
  cd "${USER_DIR}"
  rime_deployer --add-schema win32_proxy
  rime_deployer --set-active-schema win32_proxy
  rime_deployer --build "${USER_DIR}" "${SHARED_DIR}" "${STAGING_DIR}"
} 2>&1 | tee "${DEPLOY_LOG}"

if [[ "${NO_HOST}" == "1" ]]; then
  if ! python3 -c "import socket,sys; p=int(sys.argv[1]); s=socket.create_connection(('127.0.0.1',p),timeout=0.5); s.close()" "${HOST_PORT}" >/dev/null 2>&1; then
    echo "[qqpy-arif-manual] QQPY_NO_HOST=1 but no host reachable on 127.0.0.1:${HOST_PORT}" >&2
    exit 2
  fi
  echo "[qqpy-arif-manual] using existing host on 127.0.0.1:${HOST_PORT}"
else
  echo "[qqpy-arif-manual] starting host"
  HOST_CMD=(bash /opt/sogou/winabc/wine_run.sh "${HOST_DIR}/ime_host_skeleton.exe" --port "${HOST_PORT}" --dll "${DLL_PATH}")
  if [[ "${USE_XVFB}" == "1" ]]; then
    xvfb-run -a env WINEPREFIX="${WINEPREFIX_DIR}" WINEDEBUG=-all "${HOST_CMD[@]}" 2>&1 | tee "${HOST_LOG}" &
  else
    env WINEPREFIX="${WINEPREFIX_DIR}" WINEDEBUG=-all "${HOST_CMD[@]}" 2>&1 | tee "${HOST_LOG}" &
  fi
  HOST_PID=$!
  HOST_STARTED_BY_SCRIPT=1

  host_ready=0
  for _ in $(seq 1 300); do
    if python3 -c "import socket,sys; p=int(sys.argv[1]); s=socket.create_connection(('127.0.0.1',p),timeout=0.2); s.close()" "${HOST_PORT}" >/dev/null 2>&1; then
      echo "[qqpy-arif-manual] host_reachable=1" | tee -a "${HOST_LOG}"
      host_ready=1
      break
    fi
    sleep 0.1
  done

  if [[ ${host_ready} -ne 1 ]]; then
    echo "[qqpy-arif-manual] host not ready" >&2
    exit 1
  fi
fi

printf 'bind "\\"\\C-x\\C-a\\": arify-toggle"\n' > "${RC_FILE}"
printf 'echo "[qqpy-arif-manual] Press Ctrl-X Ctrl-A once to enable arify."\n' >> "${RC_FILE}"
printf 'echo "[qqpy-arif-manual] Type pinyin (e.g. ni) and press TAB to show candidates."\n' >> "${RC_FILE}"
printf 'echo "[qqpy-arif-manual] To commit a candidate, append its index then press TAB again (e.g. ni1<TAB>)."\n' >> "${RC_FILE}"
printf 'echo "[qqpy-arif-manual] Quick demo: type: echo ni<TAB>1<TAB> then Enter; expected output: 你"\n' >> "${RC_FILE}"
printf 'echo "[qqpy-arif-manual] Host: 127.0.0.1:%s"\n' "${HOST_PORT}" >> "${RC_FILE}"
printf 'PS1="arif-manual$ "\n' >> "${RC_FILE}"
chmod 600 "${RC_FILE}"

echo "[qqpy-arif-manual] entering interactive shell"

env \
  LD_PRELOAD="${PROXY_SO}" \
  ARIFY_ENGINES="${LIBARIF_RIME_SO}:arif_rime_engine" \
  ARIFY_FRONTEND=readline \
  ARIFY_PAGE_SIZE=9 \
  ARIFY_LOG_FILE="${ARIFY_LOG}" \
  ARIFY_RL_NO_AUTO_UNSETENV=1 \
  ARIF_RIME_MODULES=default,win32_proxy \
  ARIF_RIME_USER_DATA_DIR="${USER_DIR}" \
  ARIF_RIME_SHARED_DATA_DIR="${SHARED_DIR}" \
  ARIF_RIME_LOG_DIR="${USER_DIR}/log" \
  ARIF_RIME_LOG_LEVEL=INFO \
  "${ARIFY_BIN}" -p "${LIBARIFY_SO}" -f readline -- \
  bash --noprofile --rcfile "${RC_FILE}" -i

echo "[qqpy-arif-manual] shell exited"
echo "[qqpy-arif-manual] deploy_log=${DEPLOY_LOG}"
echo "[qqpy-arif-manual] arify_log=${ARIFY_LOG}"
if [[ "${HOST_STARTED_BY_SCRIPT}" == "1" ]]; then
  echo "[qqpy-arif-manual] host_log=${HOST_LOG}"
fi
