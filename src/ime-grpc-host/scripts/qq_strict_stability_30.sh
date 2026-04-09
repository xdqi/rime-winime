#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
ROOT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
cd "${ROOT_DIR}"

mkdir -p .cache

ROUNDS=${IME_STABILITY_ROUNDS:-30}
STABILITY_PHASE1_PRECHECK=${IME_STABILITY_PHASE1_PRECHECK:-0}
STAMP=$(date +%Y%m%d_%H%M%S_%N)
MASTER_LOG=".cache/qq-strict-stability-${STAMP}.log"
RESULT_TSV=".cache/qq-strict-stability-${STAMP}.tsv"

pass_count=0
fail_count=0

reason_regression_failed=0
reason_nihao_missing=0
reason_nihao_count_zero=0
reason_replay_missing=0
reason_replay_failed=0
reason_replay_chain_missing=0
reason_stale_process=0
reason_port_busy=0
reason_summary_parse=0
reason_replay_ready_timeout=0
reason_replay_early_exit=0
reason_phase1_gate_failed=0
reason_other=0

log_line() {
  echo "$*" | tee -a "${MASTER_LOG}"
}

reason_from_rc() {
  local rc="$1"
  case "${rc}" in
    10) echo "regression_failed" ;;
    11) echo "nihao_missing_target" ;;
    12) echo "nihao_candidate_count_zero" ;;
    13) echo "grpc_replay_missing" ;;
    14) echo "replay_rpc_failed" ;;
    15) echo "replay_chain_missing" ;;
    16) echo "stale_process_detected" ;;
    17) echo "port_still_busy" ;;
    18) echo "summary_parse_failed" ;;
    19) echo "replay_readiness_timeout" ;;
    20) echo "replay_host_exited_before_ready" ;;
    21) echo "phase1_gate_winimm_failed" ;;
    *) echo "other" ;;
  esac
}

count_reason() {
  local reason="$1"
  case "${reason}" in
    regression_failed) reason_regression_failed=$((reason_regression_failed + 1)) ;;
    nihao_missing_target) reason_nihao_missing=$((reason_nihao_missing + 1)) ;;
    nihao_candidate_count_zero) reason_nihao_count_zero=$((reason_nihao_count_zero + 1)) ;;
    grpc_replay_missing) reason_replay_missing=$((reason_replay_missing + 1)) ;;
    replay_rpc_failed) reason_replay_failed=$((reason_replay_failed + 1)) ;;
    replay_chain_missing) reason_replay_chain_missing=$((reason_replay_chain_missing + 1)) ;;
    stale_process_detected) reason_stale_process=$((reason_stale_process + 1)) ;;
    port_still_busy) reason_port_busy=$((reason_port_busy + 1)) ;;
    summary_parse_failed) reason_summary_parse=$((reason_summary_parse + 1)) ;;
    replay_readiness_timeout) reason_replay_ready_timeout=$((reason_replay_ready_timeout + 1)) ;;
    replay_host_exited_before_ready) reason_replay_early_exit=$((reason_replay_early_exit + 1)) ;;
    phase1_gate_winimm_failed) reason_phase1_gate_failed=$((reason_phase1_gate_failed + 1)) ;;
    *) reason_other=$((reason_other + 1)) ;;
  esac
}

log_line "[meta] rounds=${ROUNDS}"
log_line "[meta] stability_phase1_precheck=${STABILITY_PHASE1_PRECHECK}"
log_line "[meta] master_log=${MASTER_LOG}"
log_line "[meta] result_tsv=${RESULT_TSV}"
echo "round\trc\treason\tround_log" | tee -a "${RESULT_TSV}"

round=1
while [[ ${round} -le ${ROUNDS} ]]; do
  round_label=$(printf "%02d" "${round}")
  round_log=".cache/qq-strict-acceptance-round-${round_label}-${STAMP}.log"
  log_line "[round] ${round}/${ROUNDS} start"

  set +e
  IME_PHASE1_GATE_PRECHECK="${STABILITY_PHASE1_PRECHECK}" ./scripts/qq_strict_acceptance.sh 2>&1 | tee "${round_log}"
  rc=$?
  set -e

  if [[ ${rc} -eq 0 ]]; then
    pass_count=$((pass_count + 1))
    reason="pass"
    log_line "[round] ${round}/${ROUNDS} PASS"
  else
    fail_count=$((fail_count + 1))
    reason=$(reason_from_rc "${rc}")
    count_reason "${reason}"
    log_line "[round] ${round}/${ROUNDS} FAIL rc=${rc} reason=${reason} log=${round_log}"
  fi

  printf "%s\t%s\t%s\t%s\n" "${round}" "${rc}" "${reason}" "${round_log}" | tee -a "${RESULT_TSV}"

  round=$((round + 1))
done

success_rate=$(awk -v pass="${pass_count}" -v rounds="${ROUNDS}" 'BEGIN { if (rounds == 0) { printf "0.00" } else { printf "%.2f", (pass * 100.0) / rounds } }')

log_line "[summary] pass=${pass_count} fail=${fail_count} success_rate=${success_rate}%"
log_line "[summary] reason_regression_failed=${reason_regression_failed}"
log_line "[summary] reason_nihao_missing_target=${reason_nihao_missing}"
log_line "[summary] reason_nihao_candidate_count_zero=${reason_nihao_count_zero}"
log_line "[summary] reason_grpc_replay_missing=${reason_replay_missing}"
log_line "[summary] reason_replay_rpc_failed=${reason_replay_failed}"
log_line "[summary] reason_replay_chain_missing=${reason_replay_chain_missing}"
log_line "[summary] reason_stale_process_detected=${reason_stale_process}"
log_line "[summary] reason_port_still_busy=${reason_port_busy}"
log_line "[summary] reason_summary_parse_failed=${reason_summary_parse}"
log_line "[summary] reason_replay_readiness_timeout=${reason_replay_ready_timeout}"
log_line "[summary] reason_replay_host_exited_before_ready=${reason_replay_early_exit}"
log_line "[summary] reason_phase1_gate_winimm_failed=${reason_phase1_gate_failed}"
log_line "[summary] reason_other=${reason_other}"

if [[ ${fail_count} -gt 0 ]]; then
  log_line "[result] FAIL"
  exit 1
fi

log_line "[result] PASS"
