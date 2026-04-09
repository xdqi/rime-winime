# rime-grpc-proxy

New librime plugin that talks to `ime-grpc-host` using C++ gRPC.

Components:

- `grpc_key_event_processor`: sends one RPC per key event
- `grpc_proxy_translator`: queries candidates for current segment
- `grpc_commit_observer`: reports commit events back to host

Current status:

- full gRPC client skeleton is implemented
- fail-fast debug mode is supported (`debug_stop_mode: true`)
- backend failure path is intentionally aggressive for early debugging
- shared session client is reused across processor / translator / commit observer

Per-key candidate tracing (optional):

- set environment variable `IME_GRPC_TRACE_PER_KEY=1`
- proxy logs one snapshot after each successful key-down RPC
- each line includes `composition`, `reading`, candidate count and top candidates

## Contract Gate (Phase D)

Before changing proxy gRPC codegen/build wiring, run:

```bash
cd /opt/sogou/src/grpc-contract
mkdir -p .cache
./scripts/verify_single_proto_source.sh 2>&1 | tee .cache/verify-single-proto-source.log
```

## Quick Start

1. Start host server:

```bash
cd /opt/sogou/src/ime-grpc-host
cargo run
```

2. Build plugin:

```bash
cd /opt/sogou/src/rime-grpc-proxy
cmake -S . -B build
cmake --build build -j
```

3. Run per-key replay (optional, for debugging timeline):

```bash
cd /opt/sogou/src/rime-grpc-proxy
IME_GRPC_TRACE_PER_KEY=1 IME_GRPC_HOST=127.0.0.1 IME_GRPC_PORT=50096 IME_REPLAY_INPUT=nihao ./build/grpc-replay
```

Replay also supports debug-stop toggling for acceptance checks:

```bash
IME_GRPC_DEBUG_STOP_MODE=1 IME_GRPC_HOST=127.0.0.1 IME_GRPC_PORT=59999 IME_REPLAY_INPUT=nihao ./build/grpc-replay
```

Dedicated debug-stop visibility check:

```bash
cd /opt/sogou/src/rime-grpc-proxy
./scripts/qq_debug_stop_force_disable_check.sh 2>&1 | tee .cache/qq-debug-stop-force-disable.log
```

This prints one line per key and is useful for verifying incremental evolution
(`n -> ni -> nih -> niha -> nihao`) with top candidates.

4. Use schema example:

- [schema/grpc_proxy.schema.yaml](schema/grpc_proxy.schema.yaml)
- enable `grpc_key_event_processor`, `grpc_proxy_translator`, `grpc_commit_observer`

The original reverse prototype under `src/reverse/rime-win32-proxy` remains unchanged
and serves as reference implementation.