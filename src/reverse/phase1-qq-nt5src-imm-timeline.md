# Phase 1 QQ WinIMM Timeline (nt5src Alignment)

Date: 2026-04-04

## Goal

Define one stable timeline contract between:

1. Current Rust WinIMM backend behavior.
2. nt5src IMM reference path.
3. Regression evidence produced by acceptance/stability scripts.

This document is the baseline for deeper reverse work and future behavior diffs.

## nt5src Reference Anchors

1. IMM key translation pipeline:
- `nt5src/Source/XPSP1/NT/windows/core/ntuser/imm/input.c`
- focus: `ImmProcessKey -> ImmTranslateMessage -> ImeToAsciiEx -> TRANSMSG dispatch`

2. Active context and IME status notify ordering:
- `nt5src/Source/XPSP1/NT/windows/core/ntuser/imm/context.c`
- focus: `ImmSetActiveContext`, `WM_IME_SETCONTEXT`, notify open/conversion status

3. Candidate storage/read semantics:
- `nt5src/Source/XPSP1/NT/windows/core/ntuser/imm/ctxtinfo.c`
- focus: `ImmGetCandidateListCount*` and `ImmGetCandidateList*` behavior and edge cases

## Runtime Timeline Markers

Marker output target: `win_imm_timeline`

Enable with:

```bash
cd /opt/sogou/src/ime-grpc-host
RUST_LOG=ime_grpc_host=info,win_imm_timeline=info \
IME_WINIMM_TRACE_TIMELINE=1 \
IME_WINIMM_FORCE_REAL=1 \
./scripts/qq_strict_acceptance.sh 2>&1 | tee .cache/qq-strict-acceptance-timeline.log
```

### Stage Map

1. `S0_RESET_SESSION`
- code: `reset_for_new_session`
- intent: start-of-session lifecycle boundary
- nt5src relation: context activation sequence entry

2. `S1_ACTIVATE_CONTEXT`
- code: `activate_qq_ime`
- intent: apply open/conversion state and IME notifications on active context/window
- nt5src relation: `ImmSetActiveContext` + `WM_IME_SETCONTEXT` ordering

3. `S2_KEY_ENTRY`
- code: `drive_qq_ime_key`
- intent: normalized key event input (vk/scan/modifier/source)
- nt5src relation: key ingress before `ImmProcessKey`

4. `S3_IMM_PROCESS_TRANSLATE`
- code: `drive_qq_ime_key`
- intent: record `ImmProcessKey` flags and whether `ImmTranslateMessage` was run
- nt5src relation: `ImmProcessKey -> ImmTranslateMessage`

5. `S4_IME_EXPORT_CALLS`
- code: `drive_qq_ime_key`
- intent: record `ImeProcessKey` and `ImeToAsciiEx` return path with `TRANSMSG` count
- nt5src relation: core `ImeToAsciiEx` translation output

6. `S5_DISPATCH_COMPLETE`
- code: `drive_qq_ime_key`
- intent: message dispatch completion per key stroke
- nt5src relation: dispatch/post of translated IME messages

7. `S6_IMM_SNAPSHOT`
- code: `refresh_from_imm`
- intent: composition/reading/candidate snapshot after IMM reads
- nt5src relation: candidate list retrieval from IMM context

8. `S7_EVENT_RESULT`
- code: `apply_key_event`
- intent: per-key backend state output summary
- nt5src relation: end of key-processing observable state

9. `S8_QUERY_RESULT`
- code: `query_candidates`
- intent: explicit query output summary
- nt5src relation: candidate query boundary used by proxy/replay checks

## Validation Checklist

1. `qq_strict_acceptance.sh` passes.
2. Timeline log contains `S0` through `S8` in expected order for at least one key path.
3. `S3` has `process_by_ime=true` for key events where IME should consume input.
4. `S4` reports non-zero `ime_to_ascii_ex` or non-zero `trans_msg_count` during composing keys.
5. `S6` and `S8` show non-zero candidates for `nihao` in strict QQ path.

## Operational Note

`grpc-replay` default timeout is short for cold start. Acceptance script sets
`IME_GRPC_TIMEOUT_MS=3000` for replay stage to avoid first-session race failures.
