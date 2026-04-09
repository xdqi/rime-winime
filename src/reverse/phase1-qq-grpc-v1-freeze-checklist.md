# Phase 1 QQ gRPC V1 Contract Freeze Checklist (Phase A Gate)

Status: LOCKED
Freeze revision: r1
Freeze date: 2026-04-04

Source plan: /opt/sogou/src/memories/NWFjM2U2NzgtZTJlNS00YWVlLWIxYjQtZDFjNWEwNzVmYWIz/plan.md
Applies to: /opt/sogou/src/ime-grpc-host, /opt/sogou/src/rime-grpc-proxy, /opt/sogou/src/grpc-contract

## Change-Control Rules

1. Any change to RPC set, request/response key fields, or session state semantics requires:
   - updating this file,
   - incrementing `Freeze revision`,
   - re-running Phase D contract checks.
2. Any merge request touching `src/grpc-contract/proto/ime_proxy.proto` is blocked unless this checklist remains green.
3. Phase A is considered complete only when this artifact is present and LOCKED.

## Frozen Fingerprint

- Proto path: `/opt/sogou/src/grpc-contract/proto/ime_proxy.proto`
- Proto SHA256: `46104421156412396bda343e1eaff27e181f20c74e2662c223b4aaa1e67f1b13`
- Proto package: `ime.gateway.v1`
- Frozen RPC set:
  - `OpenSession`
  - `SendKeyEvent`
  - `QueryCandidates`
  - `CommitSelection`
  - `ResetSession`
  - `GetStatus`
  - `Ping`

## Phase A Freeze Checklist

### A1. RPC surface freeze

- [x] Service method set is frozen to the 7 methods listed above.
- [x] Method names and unary call shape are frozen for V1.

Evidence:
- `src/grpc-contract/proto/ime_proxy.proto` service `ImeGateway`.

### A2. Key event minimum fields freeze

- [x] `KeyEvent` carries: `seq`, `key_down`, `virtual_key`, `scan_code`, `shift`, `ctrl`, `alt`, `repeated`, `extended`, `timestamp_ms`, `source_keycode`, `source_modifier`.
- [x] Host path consumes these fields and maps them into backend key event processing.
- [x] Proxy path populates these fields on every key-down/up RPC.

Evidence:
- `src/grpc-contract/proto/ime_proxy.proto` message `KeyEvent`.
- `src/ime-grpc-host/src/main.rs` in `send_key_event` + `BackendKeyEvent` mapping.
- `src/rime-grpc-proxy/src/grpc_client.cc` in `SendKeyEvent` request population.

### A3. Session state semantics freeze

- [x] `OpenSession` must succeed before `SendKeyEvent/QueryCandidates/CommitSelection/ResetSession`.
- [x] Non-existent session returns transport-level `not_found` from host.
- [x] `SendKeyEvent` enforces monotonic sequence (`seq` must be greater than previous).
- [x] `QueryCandidates/CommitSelection` advance or preserve sequence with `max(last_seq, req.seq)` semantics.
- [x] No explicit `CloseSession` in V1; lifecycle is `idle TTL + reaper + worker release`.

Evidence:
- `src/ime-grpc-host/src/main.rs`: `open_session`, `send_key_event`, `query_candidates`, `commit_selection`, `reset_session`, `reap_idle_sessions`.

### A4. Error envelope semantics freeze

- [x] Business-level errors return gRPC success with `error_code`/`error_message` filled.
- [x] Success path returns empty `error_code`/`error_message`.
- [x] Mandatory response error codes are frozen for V1:
  - `SEQ_OUT_OF_ORDER`
  - `BACKEND_SNAPSHOT_FAILED`
  - `BACKEND_SEND_KEY_FAILED`
  - `BACKEND_QUERY_FAILED`
  - `BACKEND_COMMIT_FAILED`
  - `BACKEND_RESET_FAILED`

Evidence:
- `src/ime-grpc-host/src/main.rs` response construction in each RPC handler.

### A5. Debug-first failure semantics freeze

- [x] Proxy debug stop mode is frozen as fail-fast (`std::abort`) on transport/backend failure.
- [x] Dedicated acceptance test for debug-stop visibility is implemented and has passing baseline run.

Evidence:
- `src/rime-grpc-proxy/src/grpc_client.cc`: `FailFastIfNeeded`, `HandleStatusLocked`, `EnsureSessionLocked`.
- `src/rime-grpc-proxy/scripts/qq_debug_stop_force_disable_check.sh`.
- `src/ime-grpc-host/scripts/qq_phase_a_freeze_gate.sh` and `src/ime-grpc-host/src/bin/phase_a_contract_checker.rs`.

## Gate Decision

- Phase A gate decision: PASS (LOCKED artifact created).
- Runtime note: `IME_WORKER_BACKEND=win_imm` requires Windows host runtime; run WinIMM gates through Wine-hosted Windows binary path.
- Latest gate evidence:
  - `src/ime-grpc-host/.cache/qq-phase-a-gate-manual.log`
  - `src/ime-grpc-host/.cache/qq-phase1-gate-manual.log`
  - `src/ime-grpc-host/.cache/qq-phase1-gate-winimm-v3-manual.log`
