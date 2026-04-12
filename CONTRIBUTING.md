# Contributing

## Development Setup

See [docs/SETUP_AND_USAGE.md](docs/SETUP_AND_USAGE.md) for prerequisites and build instructions.

## Project Layout

- `src/rime-remote/` — C++ librime plugin (v3, production). Modify this for plugin-side changes.
- `src/ime-grpc-host-v2/` — Rust gRPC server. Modify this for host-side changes.
- `src/grpc-contract-v2/` — Protobuf definitions. Changes here require rebuilding both sides.
- `src/rime-grpc-proxy-v2/` — Predecessor v2 plugin. Kept for reference; not actively maintained.
- `src/reverse/` — Reverse engineering notes and proof-of-concept scripts.
- `docs/` — Technical documentation (see README for index).

## Code Style

- **Rust**: `cargo fmt` + `cargo clippy`
- **C++**: Follow existing librime style (4-space indent, `snake_case` for functions)
- **Proto**: Follow Google's protobuf style guide

## Testing

```bash
# Rust tests (requires Wine + IME DLL)
cd src/ime-grpc-host-v2
cargo test --target i686-pc-windows-gnu

# Rust tests (Linux-only, NativeRimeBackend)
cargo test
```

## Documentation

When making significant changes, update the relevant docs in `docs/`. The [LLM_CONTEXT.md](docs/LLM_CONTEXT.md) should always reflect the current production state.
