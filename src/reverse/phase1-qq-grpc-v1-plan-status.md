# Phase 1 QQ gRPC V1 Plan Alignment Status

Source of truth: "Plan: Wine QQPinyin gRPC V1 (Debug-first)"
Canonical path: /opt/sogou/src/memories/NWFjM2U2NzgtZTJlNS00YWVlLWIxYjQtZDFjNWEwNzVmYWIz/plan.md
Date: 2026-04-05

## Drift note (for control)

- Recent work over-indexed on Phase E stabilization (regression/acceptance/timeline instrumentation).
- This was useful but not a substitute for completing Phase A freeze artifact and Phase D shared proto unification.
- From now on, new tasks should be accepted only if they map to an unchecked item in this file.

## A. Contract and state semantics (freeze)

- [x] RPC set and key fields are implemented in host/proxy paths and validated by executable gate.
- [x] Session semantics (open then key/query/commit/reset) are implemented and validated by executable gate.
- [x] Formal freeze checklist doc is produced as a locked artifact.

Latest Phase A gate evidence:

- `src/ime-grpc-host/scripts/qq_phase_a_freeze_gate.sh` (PASS)
- `src/ime-grpc-host/src/bin/phase_a_contract_checker.rs` (PASS)
- `src/ime-grpc-host/.cache/qq-phase-a-gate-manual.log`
- `src/ime-grpc-host/.cache/qq-phase1-gate-manual.log`

## B. Backend architecture (Rust + tonic + worker)

- [x] Host project exists and runs as gRPC gateway.
- [x] Session to dedicated worker mapping exists.
- [x] Worker pool (min/max/prewarm/spawn timeout) exists.
- [x] Worker runtime includes WinIMM backend path.
- [x] Internal host<->worker structured IPC is in place.

## C. Frontend architecture (librime plugin)

- [x] Key processor path sends per-key events.
- [x] Translator path queries candidates and maps output.
- [x] Commit observer path is integrated.
- [x] Debug-stop behavior has dedicated acceptance coverage aligned with current plan wording.

## D. Proto and build system

- [x] Shared proto directory at /opt/sogou/src/grpc-contract is the single source of truth.
- [x] Rust/C++ codegen and build wiring now has a dedicated unification gate script.

## E. Stability and performance validation

- [x] QQ strict regression script exists and is stable.
- [x] Acceptance script (including replay chain and cleanup checks) exists.
- [x] 30-round stability baseline completed (100% pass in latest run).
- [x] Multi-frontend concurrency isolation test exists and has passing baseline run (3 clients).
- [x] Debug-stop "force disable schema" dedicated verification exists and has passing run (expected abort + visibility log).
- [x] Latency benchmark (p50/p95/p99, cold vs prewarm) exists and has baseline run.
- [x] WinIMM runtime gate baseline exists and passes via Wine-hosted Windows binary path.
- [x] Acceptance entrypoint now includes WinIMM Phase1 precheck by default (`IME_PHASE1_GATE_PRECHECK=1`), with stability-loop override (`IME_STABILITY_PHASE1_PRECHECK`).

Latest latency snapshot (stub backend, 1000 rounds):

- cold: p50=2.508ms, p95=3.694ms, p99=6.631ms
- prewarm: p50=2.533ms, p95=3.808ms, p99=5.342ms
- note: benchmark readiness now uses passive port checks to avoid consuming the prewarmed worker before measurement.

Latest WinIMM gate evidence:

- `src/ime-grpc-host/.cache/qq-phase1-gate-winimm-v3-manual.log`
- `src/ime-grpc-host/.cache/qq-phase-a-gate-winimm-run-20260404_213841_941128658.log`

Latest replay/WinIMM baseline (2026-04-05):

- `src/ime-grpc-host/.cache/qq-strict-acceptance-winimm-baseline-20260405_071255_557906665.log` (PASS, replay chain passed)
- `src/ime-grpc-host/.cache/qq-phase1-gate-winimm-baseline-20260405_071431_912807229.log` (PASS)

## Immediate return-to-plan actions

1. [x] Produce a formal freeze artifact for Phase A and treat it as a gate.
2. [x] Complete Phase D by making /opt/sogou/src/grpc-contract the sole proto source.
3. [x] Add missing Phase E tests: concurrency isolation, debug-stop visibility, latency benchmark.
