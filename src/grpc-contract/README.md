# gRPC Contract

This directory stores shared protobuf contracts for the IME gateway.

- `proto/ime_proxy.proto` is used by:
  - `src/ime-grpc-host` (Rust + tonic server)
  - `src/rime-grpc-proxy` (C++ gRPC client plugin)

The package namespace is `ime.gateway.v1`.

## Phase D Gate: Single Proto Source

Run the contract gate check before merging contract/build changes:

```bash
cd /opt/sogou/src/grpc-contract
mkdir -p .cache
./scripts/verify_single_proto_source.sh 2>&1 | tee .cache/verify-single-proto-source.log
```

The check fails if:

- more than one `.proto` exists under `/opt/sogou/src` (excluding build/cache artifacts)
- `ime-grpc-host/build.rs` is not wired to `../grpc-contract/proto/ime_proxy.proto`
- `rime-grpc-proxy/CMakeLists.txt` is not wired to `../grpc-contract/proto/ime_proxy.proto`