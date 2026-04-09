#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export QQPY_INPUTS_CSV="ni,hao,zhong"
export IME_POOL_MIN_IDLE=3
exec "${SCRIPT_DIR}/run_arif_tab_smoke_grpc.sh"
