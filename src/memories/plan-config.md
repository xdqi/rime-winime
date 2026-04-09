## Plan: Distributed gRPC Setup and Weasel Integration

This plan details the configuration options for the frontend (Weasel plugin) and backend (Rust host), and the strategy for integration.

**Steps**
1. **Configurable Backend for Proxy (C++)**
   - Update `GrpcKeyEventProcessor` constructor in the C++ proxy to read configuration from its schema (`grpc_proxy.schema.yaml`).
   - Extract the following configurations:
     - `grpc_proxy/backend_address`: (e.g., `"127.0.0.1:50051"`)
     - `grpc_proxy/rpc_timeout_ms`: Critical to prevent the Weasel UI/Windows thread from locking up if the backend RPC hangs (e.g. `50`).
     - `grpc_proxy/fallback_on_error`: Boolean to control whether to simply bypass processing when gRPC fails, falling back to English typing.
   - Update `GrpcImeClientV2::Instance(...)` to respect these settings (address and deadline for calls).

2. **Configurable Rust Host (`ime-grpc-host-v2`)**
   - Add the `clap` crate to `Cargo.toml`.
   - Update `src/main.rs` to parse a command-line arguments (with environment variable fallbacks):
     - `--bind` / `GRPC_BIND_ADDR`: The IP:port to listen on (default `"127.0.0.1:50051"`).
     - `--ime-path` / `GRPC_IME_PATH`: The absolute or relative path to the target IME DLL (e.g. `sys/SogouPY.ime`). Currently hardcoded to `C:\windows\system32\QQPinyin.ime`.
     - `--show-window`: Boolean flag. By default, the host creates a hidden `HWND_MESSAGE` window (per user preferences). This flag can force setting `WS_VISIBLE` for debugging UI popups from the loaded IME DLL.
     - `--session-timeout-sec`: Since Weasel instances can crash without sending a close request, the Rust host should have an idle timer to destroy the UI window and unload the DLL for stale sessions.

3. **Weasel Integration Strategy**
   - Build `rime-grpc-proxy-v2` as a dynamic library.
   - **For now:** Manually copy the built DLL into Weasel's Rime `plugins\` directory for testing.
   - Deploy `grpc_proxy.schema.yaml` to the Rime user data folder (`%AppData%\Rime\`).
   - Modify the user's `default.custom.yaml` to enable the proxy schema.

**Relevant files**
- [src/rime-grpc-proxy-v2/src/grpc_key_event_processor.cc](src/rime-grpc-proxy-v2/src/grpc_key_event_processor.cc) — Read configurations via `ticket.engine->schema()->config()`.
- [src/rime-grpc-proxy-v2/src/grpc_client.cc](src/rime-grpc-proxy-v2/src/grpc_client.cc) — Apply timeouts to `grpc::ClientContext::set_deadline()` and remove hardcoded address.
- [src/rime-grpc-proxy-v2/grpc_proxy.schema.yaml](src/rime-grpc-proxy-v2/grpc_proxy.schema.yaml) — Define the configuration schema defaults.
- [src/ime-grpc-host-v2/Cargo.toml](src/ime-grpc-host-v2/Cargo.toml) — Add `clap` dependency.
- [src/ime-grpc-host-v2/src/main.rs](src/ime-grpc-host-v2/src/main.rs) — Implement `clap::Parser`.
- [src/ime-grpc-host-v2/src/win_imm/mod.rs](src/ime-grpc-host-v2/src/win_imm/mod.rs) — Replace hardcoded `QQPinyin.ime` with the injected configured path, and handle the `--show-window` flag.

**Verification**
1. Test configuring `grpc_proxy.schema.yaml` with a remote IP (e.g. `127.0.0.1`), artificially delay the host, and confirm Weasel doesn't freeze due to `rpc_timeout_ms`.
2. Run `ime-grpc-host-v2.exe --bind 127.0.0.1:22222 --ime-path sys/SogouPY.ime` and verify it correctly targets the Sogou DLL instead of QQPinyin.
3. Observe standard hidden window behavior in `host 测试` environment, confirming consistency with preferences unless `--show-window` is provided.

**Decisions**
- Chose schema-level configuration for the C++ proxy to cleanly handle timeouts natively within Rime's engine lifecycle.
- Chose `clap` args + env fallback for the Rust host.
- Decided to add a session timeout in Rust to defend against zombie sessions (memory leaks) during rapid Weasel restart cycles.