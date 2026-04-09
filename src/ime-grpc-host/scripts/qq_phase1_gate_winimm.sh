#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
CONTRACT_DIR=$(cd "${ROOT_DIR}/../grpc-contract" && pwd)
cd "${ROOT_DIR}"

mkdir -p .cache

STAMP=$(date +%Y%m%d_%H%M%S_%N)
RUN_LOG=".cache/qq-phase1-gate-winimm-${STAMP}.log"

log_line() {
  echo "$*" | tee -a "${RUN_LOG}"
}

run_step() {
  local name="$1"
  shift

  log_line "[step] start ${name}"
  set +e
  "$@" 2>&1 | tee -a "${RUN_LOG}"
  local rc=$?
  set -e
  if [[ ${rc} -ne 0 ]]; then
    log_line "[step] FAIL ${name} rc=${rc}"
    log_line "[result] FAIL"
    log_line "[summary] run_log=${RUN_LOG}"
    exit "${rc}"
  fi
  log_line "[step] PASS ${name}"
}

run_step "phase_d_single_proto_source" "${CONTRACT_DIR}/scripts/verify_single_proto_source.sh"
run_step "phase_a_contract_semantics_winimm" "${ROOT_DIR}/scripts/qq_phase_a_freeze_gate_winimm.sh"

log_line "[result] PASS"
log_line "[summary] run_log=${RUN_LOG}"
