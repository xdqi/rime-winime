# Phase1 WinIMM Baseline Archive (2026-04-05)

## Scope

- Task 1: replay timeout convergence in strict acceptance
- Task 2: standalone WinIMM Phase1 gate baseline

## Commands Executed

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_strict_acceptance.sh 2>&1 | tee .cache/qq-strict-acceptance-winimm-baseline-20260405_071255_557906665.log
```

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_phase1_gate_winimm.sh 2>&1 | tee .cache/qq-phase1-gate-winimm-baseline-20260405_071431_912807229.log
```

## Result

- strict acceptance: PASS
- replay chain (`n -> ni -> nih -> niha -> nihao`): PASS
- standalone WinIMM Phase1 gate: PASS

## Evidence Logs

- `src/ime-grpc-host/.cache/qq-strict-acceptance-winimm-baseline-20260405_071255_557906665.log`
- `src/ime-grpc-host/.cache/qq-phase1-gate-winimm-baseline-20260405_071431_912807229.log`
- `src/ime-grpc-host/.cache/qq-phase-a-gate-winimm-run-20260405_071432_317385836.log`
- `src/ime-grpc-host/.cache/qq-phase-a-gate-winimm-host-20260405_071432_317385836.log`
- `src/ime-grpc-host/.cache/qq-phase-a-gate-winimm-check-20260405_071432_317385836.log`

## Notes

- Acceptance entrypoint now performs WinIMM Phase1 precheck by default (`IME_PHASE1_GATE_PRECHECK=1`).
- Stability loop keeps precheck optional via `IME_STABILITY_PHASE1_PRECHECK` to avoid per-round overhead.
