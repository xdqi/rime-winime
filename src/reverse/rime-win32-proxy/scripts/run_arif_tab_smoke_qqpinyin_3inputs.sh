#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROXY_DIR="$(cd "${SCRIPT_DIR}/.." && pwd)"
ROOT_DIR="$(cd "${PROXY_DIR}/../../.." && pwd)"
CACHE_DIR="${ROOT_DIR}/.cache"
RUNNER="${SCRIPT_DIR}/run_arif_tab_smoke_qqpinyin.sh"

TS="$(date +%Y%m%d_%H%M%S)"
LOOP_LOG="${CACHE_DIR}/rime_win32_proxy_tab_smoke_3inputs_${TS}.log"
INPUTS_CSV="${QQPY_INPUTS_CSV:-ni,hao,zhong}"

mkdir -p "${CACHE_DIR}"

if [[ ! -x "${RUNNER}" ]]; then
  echo "[qqpy-arif-tab-3inputs] missing executable runner: ${RUNNER}" >&2
  exit 2
fi

IFS=',' read -r -a INPUTS <<< "${INPUTS_CSV}"
if [[ ${#INPUTS[@]} -ne 3 ]]; then
  echo "[qqpy-arif-tab-3inputs] QQPY_INPUTS_CSV must contain exactly 3 comma-separated items" >&2
  echo "[qqpy-arif-tab-3inputs] got: ${INPUTS_CSV}" >&2
  exit 2
fi

trim_spaces() {
  local s="$1"
  s="${s#"${s%%[![:space:]]*}"}"
  s="${s%"${s##*[![:space:]]}"}"
  printf '%s' "$s"
}

repeat_backspace_token() {
  local count="$1"
  local out=""
  local i
  for ((i = 0; i < count; ++i)); do
    out+="<退格>"
  done
  printf '%s' "$out"
}

for i in 0 1 2; do
  INPUTS[$i]="$(trim_spaces "${INPUTS[$i]}")"
  if [[ -z "${INPUTS[$i]}" ]]; then
    echo "[qqpy-arif-tab-3inputs] input #$((i + 1)) is empty after trimming" >&2
    exit 2
  fi
done

{
  echo "[qqpy-arif-tab-3inputs] log=${LOOP_LOG}"
  echo "[qqpy-arif-tab-3inputs] inputs=${INPUTS[0]},${INPUTS[1]},${INPUTS[2]}"
} | tee -a "${LOOP_LOG}"

echo "[qqpy-arif-tab-3inputs] single_process=1" | tee -a "${LOOP_LOG}"
echo "[qqpy-arif-tab-3inputs] key_sequence=${INPUTS[0]}$(repeat_backspace_token "${#INPUTS[0]}")${INPUTS[1]}$(repeat_backspace_token "${#INPUTS[1]}")${INPUTS[2]}" | tee -a "${LOOP_LOG}"

QQPY_INPUT_TEXT="${INPUTS[0]}" \
QQPY_INPUTS_CSV="${INPUTS[0]},${INPUTS[1]},${INPUTS[2]}" \
"${RUNNER}" 2>&1 | tee -a "${LOOP_LOG}"

echo "[qqpy-arif-tab-3inputs] done" | tee -a "${LOOP_LOG}"
echo "[qqpy-arif-tab-3inputs] loop_log=${LOOP_LOG}" | tee -a "${LOOP_LOG}"
