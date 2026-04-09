# ime-grpc-host

Rust + tonic gRPC gateway for the Wine-side IME worker runtime.

## Environment Variables

- `IME_GRPC_BIND` (default: `127.0.0.1:50051`)
- `IME_SESSION_IDLE_TTL_SECS` (default: `30`)
- `IME_POOL_MIN_IDLE` (default: `1`)
- `IME_POOL_MAX_IDLE` (default: `4`)
- `IME_POOL_PREWARM` (default: `true`)
- `IME_POOL_SPAWN_TIMEOUT_MS` (default: `1500`)
- `IME_WINIMM_FORCE_REAL` (default: `0`)
- `IME_WINIMM_DLL` (default: `C:\windows\system32\QQPinyin.ime`)
- `IME_WINIMM_TRACE_TIMELINE` (default: `0`, emits nt5src-aligned timeline markers)

## Run

```bash
cargo run --bin ime-grpc-host
```

## Contract Gate (Phase D)

Before changing host gRPC codegen/build wiring, run:

```bash
cd /opt/sogou/src/grpc-contract
mkdir -p .cache
./scripts/verify_single_proto_source.sh 2>&1 | tee .cache/verify-single-proto-source.log
```

## Contract/State Gate (Phase A)

Run the formal Phase A checker (RPC contract + session state semantics):

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_phase_a_freeze_gate.sh 2>&1 | tee .cache/qq-phase-a-gate.log
```

## One-Click Phase 1 Gate (Phase D + Phase A)

Run shared proto source gate and Phase A gate in one command:

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_phase1_gate.sh 2>&1 | tee .cache/qq-phase1-gate.log
```

## WinIMM Phase A/Phase1 Gates (Wine runtime)

Run Phase A gate against the real WinIMM backend path (Windows host under Wine):

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_phase_a_freeze_gate_winimm.sh 2>&1 | tee .cache/qq-phase-a-gate-winimm.log
```

Run one-click Phase1 gate against WinIMM runtime:

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_phase1_gate_winimm.sh 2>&1 | tee .cache/qq-phase1-gate-winimm.log
```

## Smoke Test

In another terminal, run:

```bash
cd /opt/sogou/src/ime-grpc-host
cargo run --bin smoke_client
```

Optional env vars:

- `IME_GRPC_ENDPOINT` (default `http://127.0.0.1:50051`)
- `IME_SMOKE_INPUT` (default `rime`)

## Cross-Target Check (Windows GNU x86)

Recommended pre-integration check:

```bash
rustup target add i686-pc-windows-gnu
cd /opt/sogou/src/ime-grpc-host
cargo build --target i686-pc-windows-gnu --bins
```

Expected artifacts:

- `target/i686-pc-windows-gnu/debug/ime-grpc-host.exe`
- `target/i686-pc-windows-gnu/debug/smoke_client.exe`

This initial version is a functional skeleton:

- maintains session table
- allocates per-session worker handles from a configurable pool
- exposes all v1 RPC methods
- implements deterministic fake candidates for integration testing
- uses subprocess worker IPC over length-prefixed framed stdio JSON

## IPC Reliability Note

Worker IPC uses framed binary transport on stdio (`u32 length + JSON payload`) instead of
newline-delimited text. This avoids line-buffer ambiguity and is generally reliable on
Windows anonymous pipes for parent-child communication.

If we need cross-process discovery or external worker attach in the future, we can add
named pipe transport as an additional backend without changing gRPC contracts.

Real IMM worker process orchestration will be plugged into the worker pool path.

## QQ Strict Regression and Acceptance

Run one strict regression pass (host ready + smoke inputs):

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_strict_regression.sh 2>&1 | tee .cache/qq-strict-regression-manual.log
```

Run one acceptance pass (regression + per-key replay chain + cleanup checks):

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_strict_acceptance.sh 2>&1 | tee .cache/qq-strict-acceptance-manual.log
```

`qq_strict_acceptance.sh` now runs WinIMM Phase1 precheck by default.

- Default: `IME_PHASE1_GATE_PRECHECK=1`
- Skip precheck (faster local loop): `IME_PHASE1_GATE_PRECHECK=0`

Replay stability knobs (used when replay target is slow to start):

- `IME_REPLAY_READY_MAX_ATTEMPTS` (default `60`)
- `IME_REPLAY_RPC_TIMEOUT_MS` (default `8000`)
- `IME_REPLAY_POOL_MIN_IDLE` (default `1`)

Run 30-round stability baseline:

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_strict_stability_30.sh 2>&1 | tee .cache/qq-strict-stability-30.log
```

Stability loop defaults to skipping per-round Phase1 precheck to avoid 30x overhead.

- Default: `IME_STABILITY_PHASE1_PRECHECK=0`
- Enable per-round precheck: `IME_STABILITY_PHASE1_PRECHECK=1`

## Phase E Dedicated Checks

Multi-client concurrency isolation (default 3 clients on one host):

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_concurrency_isolation.sh 2>&1 | tee .cache/qq-concurrency-isolation.log
```

Latency benchmark (cold vs prewarm, outputs p50/p95/p99):

```bash
cd /opt/sogou/src/ime-grpc-host
./scripts/qq_latency_benchmark.sh 2>&1 | tee .cache/qq-latency-benchmark.log
```

Useful env vars:

- `IME_CONCURRENCY_CLIENTS` (default `3`)
- `IME_CONCURRENCY_INPUTS` (default `"nihao rime abc"`)
- `IME_BENCH_ROUNDS` (default `1000`)
- `IME_LATENCY_TARGET_P95_MS` (default `10`)
- `IME_LATENCY_ENFORCE_TARGET` (default `0`, set `1` to fail when target is exceeded)

## WinIMM Timeline Trace (NT5src Alignment)

Timeline markers are emitted under tracing target `win_imm_timeline` and map to the
IMM pipeline reference in `nt5src`.

Enable trace markers:

```bash
cd /opt/sogou/src/ime-grpc-host
RUST_LOG=ime_grpc_host=info,win_imm_timeline=info \
IME_WINIMM_TRACE_TIMELINE=1 \
IME_WINIMM_FORCE_REAL=1 \
./scripts/qq_strict_acceptance.sh 2>&1 | tee .cache/qq-strict-acceptance-timeline.log
```