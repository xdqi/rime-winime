# LLM Context Manual — Sogou IME gRPC Bridge

> Optimized for AI coding assistants. Covers the **current production stack only**.
> Last updated: 2026-04-12

---

## Project Identity

This project bridges closed-source Windows IME engines (QQPinyin, Sogou Pinyin) to the Rime input method framework via gRPC. The Rime plugin side runs inside native Rime frontends — **Weasel** (Windows), **Squirrel** (macOS), or **fcitx5-rime / ibus-rime** (Linux). The server side runs the IME DLL either natively (Windows/Weasel) or under Wine (Linux/macOS).

The current production stack has three components:

- **rime-remote** (v3) — C++ librime plugin using standard Processor/Segmentor/Translator pipeline. Successor to `rime-grpc-proxy-v2` (v2), which used Rime C API hooking.
- **ime-grpc-host-v2** — Rust gRPC server that loads a Windows `.ime` DLL through Win32 IMM APIs. Supports both 32-bit (`i686-pc-windows-gnu`, for QQPinyin and Sogou 10.5b) and 64-bit (`x86_64-pc-windows-gnu`, for Sogou 16.3) targets.
- **grpc-contract-v2** — Protobuf service definition (`rime_service.proto`, package `rime.service.v2`).

The superseded **rime-grpc-proxy-v2** (v2) is still present in the source tree. It works but has architectural limitations (monolithic `grpc_proxy_module.cc`, no ascii_composer support, stdbool ABI workaround for Squirrel).

---

## Architecture Diagram

```
 Rime Frontend (any platform)             Wine / Windows (32-bit or 64-bit)
 ─────────────────────────────────────   ────────────────────────────────────
 ┌──────────────┐                        ┌─────────────────────────┐
 │  Application │                        │  ime-grpc-host-v2.exe   │
 │  (terminal,  │                        │  (Rust + tonic gRPC)    │
 │   editor)    │                        │                         │
 └──────┬───────┘                        │  ┌──────────────────┐   │
        │ keystrokes                     │  │  ImmRimeAdapter   │   │
 ┌──────▼───────┐                        │  │  ┌────────────┐  │   │
 │   librime    │                        │  │  │ WinImmSess. │  │   │
 │  (engine)    │                        │  │  │ HWND + HIMC │  │   │
 │              │      gRPC over TCP     │  │  └──────┬─────┘  │   │
 │ ┌──────────┐ │◄──────────────────────►│  │         │ IMM DDI│   │
 │ │  rime-   │ │   rime.service.v2      │  │  ┌──────▼─────┐  │   │
 │ │  remote  │ │   :50051               │  │  │ QQPinyin/  │  │   │
 │ │  (v3)    │ │                        │  │  │ SogouPY    │  │   │
 │ └──────────┘ │                        │  │  │ .ime DLL   │  │   │
 └──────────────┘                        │  └──────────────────┘   │
   Weasel / Squirrel /                   └─────────────────────────┘
   fcitx5-rime / ibus-rime
```

**Data flow per keystroke:**
1. User presses key in application
2. librime dispatches to `rime-remote` plugin's `RemoteProcessor`
3. `RemoteProcessor` calls `GrpcImeClientV2::ProcessKey(session, keycode, mask)`
4. gRPC request reaches `ime-grpc-host-v2` → `RimeServerImpl::process_key()`
5. Server delegates to `ImmRimeAdapter::process_key()` (Windows) or `NativeRimeBackend` (Linux)
6. On Windows: `ImeProcessKey()` + `ImeToAsciiEx()` + `ImmGetCompositionString()` via IME DLL
7. Response returns accepted/rejected flag
8. If accepted, `RemoteProcessor` calls `GetContext()` → composition + candidates returned
9. `RemoteTranslator` converts candidates into Rime `Candidate` objects
10. If commit detected, `GetCommit()` returns committed text

---

## Component Registry

### grpc-contract-v2

| Item | Value |
|---|---|
| Path | `src/grpc-contract-v2/` |
| Language | Protobuf |
| Purpose | Canonical gRPC service definition for the v2 protocol |
| Key file | `proto/rime_service.proto` |
| Package | `rime.service.v2` |
| Service | `RimeService` (6 RPCs) |

**RPC methods:**
- `OpenSession(OpenSessionRequest) → OpenSessionResponse`
- `ProcessKey(ProcessKeyRequest) → ProcessKeyResponse`
- `GetContext(GetContextRequest) → GetContextResponse`
- `GetCommit(GetCommitRequest) → GetCommitResponse`
- `SelectCandidateOnCurrentPage(SelectCandidateRequest) → SelectCandidateResponse`
- `DestroySession(DestroySessionRequest) → DestroySessionResponse`

**Key message types:**
- `KeyEvent { int32 keycode; int32 modifier; }` — Rime-style keysyms (XKB), not Win32 VK
- `CandidateProto { string text; string comment; }` — single candidate item
- `MenuProto { int32 page_size; int32 page_no; bool is_last_page; int32 highlighted_candidate_index; repeated CandidateProto candidates; }`
- `CompositionProto { string preedit; int32 cursor_pos; int32 sel_start; int32 sel_end; string commit_text_preview; }`
- `RimeContextProto { CompositionProto composition; MenuProto menu; string commit_text_preview; }`

### ime-grpc-host-v2

| Item | Value |
|---|---|
| Path | `src/ime-grpc-host-v2/` |
| Language | Rust |
| Build | Cargo (`cargo build` / `cargo build --target i686-pc-windows-gnu`) |
| Purpose | gRPC server hosting Win32 IME or native Rime backend |
| Binary | `ime-grpc-host-v2[.exe]` |
| Dependencies | tonic, prost, tokio, clap, tracing, windows (crate), xkeysym |

**Source files:**

| File | Purpose |
|---|---|
| `src/main.rs` | CLI entry point (clap). Parses `--bind`, `--ime-path`. Selects backend. Starts tonic server. |
| `src/lib.rs` | Module declarations: `pub mod backend`, `pub mod win_imm` (cfg windows), `pub mod server`, proto include. |
| `src/server.rs` | `RimeServerImpl` struct wrapping `Arc<Mutex<dyn RimeBackend>>`. Implements all 6 RPCs. |
| `src/client.rs` | Demo gRPC client for manual testing. |
| `src/test_client.rs` | Minimal client binary. |
| `src/backend/mod.rs` | `RimeBackend` async trait definition. |
| `src/backend/native.rs` | `NativeRimeBackend` — Linux-only, links to `librime` via FFI. |
| `src/backend/rime_ffi.rs` | Raw C FFI declarations for `rime_api_t` functions. |
| `src/win_imm/mod.rs` | `ImmRimeAdapter` — main Win32 IMM adapter (~410 lines). Session management, `process_vk()`, `get_context()`, `get_commit()`, `select_candidate()`, paired punctuation splitting. |
| `src/win_imm/imm_ops.rs` | Low-level IMM FFI (~300 lines). `ImeFunctions` struct with fn pointers. `load_ime_dll()`, `get_composition_string()`, `get_result_string()`, `get_candidate_list()`. |
| `src/win_imm/session.rs` | `WinImmSession` — per-session hidden HWND + HIMC. Creates invisible window with custom WndProc. |
| `src/win_imm/vk_map.rs` | `rime_to_vk()` — maps Rime XKB keysyms to Win32 `VIRTUAL_KEY`. `make_l_key_data()` for lParam. `is_shifted_char()`. |
| `src/win_imm/punct_map.rs` | `ascii_to_fullwidth_punct()` — fallback table for when IME does not handle standalone punctuation. |
| `src/win_imm/channel_adapter.rs` | `ChannelImmAdapter` — isolates Win32 calls to a dedicated thread via `tokio::sync::oneshot` channels. |
| `src/win_imm/keys.rs` | Virtual key injection stub (placeholder). |
| `src/win_imm/thread_pump.rs` | Win32 message pump stub (placeholder). |
| `build.rs` | tonic-build proto compilation from `../grpc-contract-v2/proto/rime_service.proto`. |
| `tests/test_imm.rs` | Integration tests: nihao, multi-commit, uppercase passthrough, punctuation, mixed `23:59`. |
| `tests/test_grpc_punctuation.rs` | gRPC-level punctuation round-trip test. |
| `tests/test_version_detect.rs` | Windows PE version detection logic test. |

**Key types and their roles:**

```rust
// src/backend/mod.rs
#[tonic::async_trait]
pub trait RimeBackend: Send + Sync {
    async fn open_session(&self) -> Result<String>;
    async fn destroy_session(&self, session_id: &str) -> Result<()>;
    async fn process_key(&self, session_id: &str, keycode: i32, mask: i32) -> Result<bool>;
    async fn get_context(&self, session_id: &str) -> Result<Option<RimeContextProto>>;
    async fn get_commit(&self, session_id: &str) -> Result<Option<String>>;
    async fn select_candidate(&self, session_id: &str, index: i32) -> Result<bool>;
}

// src/win_imm/mod.rs
pub struct ImmRimeAdapter {
    ime_funcs: Arc<ImeFunctions>,    // loaded DLL function pointers
    sessions: Mutex<HashMap<String, WinImmSession>>,
}

// src/win_imm/imm_ops.rs
pub struct ImeFunctions {
    pub ime_inquire: ImeInquireFn,
    pub ime_select: ImeSelectFn,
    pub ime_process_key: ImeProcessKeyFn,
    pub ime_to_ascii_ex: ImeToAsciiExFn,
    // ...loaded from .ime DLL via GetProcAddress
}

// src/win_imm/session.rs
pub struct WinImmSession {
    pub hwnd: HWND,            // hidden window for message pump
    pub himc: HIMC,            // input method context
    pub pending_commit: Option<String>,  // buffered committed text
}
```

### rime-remote

| Item | Value |
|---|---|
| Path | `src/rime-remote/` |
| Language | C++17 |
| Build | CMake (standalone shared library or librime-integrated) |
| Purpose | librime plugin: delegates ALL input processing to remote gRPC backend |
| Output | `librime-remote.so` / `rime-remote.dll` |
| Dependencies | librime, gRPC C++, protobuf |

**Source files:**

| File | Purpose |
|---|---|
| `src/remote_module.cc` | RIME_REGISTER_MODULE. Registers RemoteProcessor, RemoteSegmentor, RemoteTranslator. |
| `src/remote_processor.h/cc` | `RemoteProcessor` — key event handler (~365 lines). Intercepts all keys, forwards via gRPC. Handles ASCII mode toggle (Shift commit styles). Manages session lifecycle. |
| `src/remote_segmentor.h/cc` | `RemoteSegmentor` — tags entire input as `"remote"` segment so RemoteTranslator handles it. |
| `src/remote_translator.h/cc` | `RemoteTranslator` — calls `GetContext()`, converts candidates to Rime `SimpleCandidate` objects with select keys. |
| `src/shared_state.h` | `SharedState` (~130 lines) — cross-component singleton per engine. Holds `GrpcImeClientV2` reference, backend address, timeout, session ID, cached `RimeContextProto`, v-mode regex, commit buffer. |
| `CMakeLists.txt` | Dual-mode build: standalone (`find_package(PkgConfig)`) or librime-integrated (`rime_library` variable). Generates proto C++ code. |
| `remote.schema.yaml` | Configuration: `backend_address`, `rpc_timeout_ms`, `v_mode_preedit_regex`, `ascii_composer` switch keys. |

**Key types:**

```cpp
// shared_state.h
struct SharedState {
    std::shared_ptr<GrpcImeClientV2> client;
    std::string backend_address;        // "127.0.0.1:50051"
    int rpc_timeout_ms;                 // 200
    std::string remote_session_id;      // gRPC session bound to Rime session
    rime::service::v2::RimeContextProto cached_context;
    std::regex v_mode_regex;            // "^v\\d" for Sogou v-mode detection
    std::string pending_commit;         // buffered committed text
};

// GrpcImeClientV2 (from rime-grpc-proxy-v2/src/grpc_client.h, shared code)
class GrpcImeClientV2 {
    static std::shared_ptr<GrpcImeClientV2> GetOrCreate(addr, timeout);
    bool OpenSession(uintptr_t rime_session, const std::string& schema_id);
    bool ProcessKey(uintptr_t session, int keycode, int mask);
    bool GetContext(uintptr_t session, RimeContextProto* out);
    bool GetCommit(uintptr_t session, std::string* out);
    bool SelectCandidate(uintptr_t session, int index);
    void DestroySession(uintptr_t session);
};
```

---

## Build Matrix

| Component | Target | Command | Output |
|---|---|---|---|
| ime-grpc-host-v2 (Linux, Rime backend) | `x86_64-unknown-linux-gnu` | `cargo build --release` | `ime-grpc-host-v2` |
| ime-grpc-host-v2 (Windows 32-bit, for QQPinyin / Sogou 10.5b) | `i686-pc-windows-gnu` | `cargo build --release --target i686-pc-windows-gnu` | `ime-grpc-host-v2.exe` (32-bit) |
| ime-grpc-host-v2 (Windows 64-bit, for Sogou 16.3) | `x86_64-pc-windows-gnu` | `cargo build --release --target x86_64-pc-windows-gnu` | `ime-grpc-host-v2.exe` (64-bit) |
| rime-remote (Weasel, Windows) | x64-windows-static | `$env:RIME_PLUGINS="rime-remote"; build.bat release` | Linked into `rime.dll` |
| rime-remote (Squirrel, macOS) | macOS x86_64/arm64 | Built via librime Makefile with gRPC (homebrew) | `librime-remote.dylib` |
| rime-remote (standalone, Linux) | Linux x86_64 | `cmake -B build && cmake --build build` | `librime-remote.so` |
| rime-remote (librime-integrated, Linux) | Linux x86_64 | Built as part of librime with `rime_library` set | Linked into `librime.so` |
| Proto codegen (Rust) | N/A | Automatic via `build.rs` + tonic-build | `rime.service.v2` module |
| Proto codegen (C++) | N/A | CMake custom command via `protoc` + `grpc_cpp_plugin` | `rime_service.pb.{h,cc}`, `rime_service.grpc.pb.{h,cc}` |

**Architecture note:** The host binary architecture must match the IME DLL bitness:
- **32-bit** (`i686-pc-windows-gnu`) — for QQPinyin 6.6 and Sogou 10.5b (XP edition). Uses Wine32 prefix.
- **64-bit** (`x86_64-pc-windows-gnu`) — for Sogou 16.3 (PE32+ x64, `SogouPY.ime`). Uses Wine64 prefix. Note: WeType was considered but not usable — it is pure TSF/TIP with no IMM API.

Both targets run under Wine on Linux/macOS, or natively on Windows (inside Weasel).

---

## Win32 IMM Subsystem Details

### DLL Loading Flow (`imm_ops.rs`)

```
load_ime_dll(path: "C:\\windows\\system32\\QQPinyin.ime")
  → LoadLibraryW(path)
  → GetProcAddress("ImeInquire")   → ImeFunctions.ime_inquire
  → GetProcAddress("ImeSelect")    → ImeFunctions.ime_select
  → GetProcAddress("ImeProcessKey")→ ImeFunctions.ime_process_key
  → GetProcAddress("ImeToAsciiEx") → ImeFunctions.ime_to_ascii_ex
  → GetProcAddress("NotifyIME")    → ImeFunctions.notify_ime
  → GetProcAddress("ImeSetCompositionString") → ...
```

### Key Processing Pipeline (`ImmRimeAdapter::process_vk`)

```
1. Convert Rime keysym → Win32 VK + scan code     (vk_map.rs)
2. Build lParam with scan code and flags           (make_l_key_data)
3. Call ImeProcessKey(himc, vk, lParam)            (imm_ops.rs)
4. If accepted:
   a. Call ImeToAsciiEx(vk, scancode, keystates, &trans_msgs, 0, himc)
   b. Pump WM_IME_COMPOSITION/WM_CHAR messages from trans_msgs
   c. Query ImmGetCompositionString(himc, GCS_COMPSTR) → preedit
   d. Query ImmGetCompositionString(himc, GCS_RESULTSTR) → committed text
   e. Query ImmGetCandidateList(himc, 0) → candidate list
5. If rejected and is standalone punctuation:
   a. Fallback: ascii_to_fullwidth_punct()          (punct_map.rs)
```

### VK Mapping (`vk_map.rs`)

Rime frontend uses XKB-style keysyms (e.g., `0x61` for 'a', `0xff0d` for Return). Windows IMM expects `VIRTUAL_KEY` codes. The `rime_to_vk()` function maps:

- ASCII lowercase `a-z` → `VK_A..VK_Z` (0x41–0x5A), with `is_shifted = false`
- ASCII uppercase `A-Z` → `VK_A..VK_Z`, with `is_shifted = true`
- ASCII digits `0-9` → `VK_0..VK_9`
- Punctuation: mapped individually (`,` → `VK_OEM_COMMA`, etc.)
- Special keys: `Return` → `VK_RETURN`, `BackSpace` → `VK_BACK`, `Escape` → `VK_ESCAPE`, `Tab` → `VK_TAB`, `space` → `VK_SPACE`, arrows, page up/down, home/end

### Punctuation Fallback (`punct_map.rs`)

When IME does not populate `GCS_RESULTSTR` for standalone punctuation keys (common with certain IMEs), a static table maps ASCII punctuation to Chinese fullwidth equivalents:

```
,  → ，    .  → 。    ?  → ？    !  → ！    :  → ：    ;  → ；
"  → ""   '  → ''    (  → （    )  → ）    <  → 《    >  → 》
```

Paired punctuation (`""`, `''`, `《》`, `（）`) toggles between opening and closing forms.

### Session Lifecycle (`session.rs`)

```
WinImmSession::create(ime_funcs)
  → CreateWindowExW(0, "STATIC", ..., HWND_MESSAGE)  // hidden message-only window
  → ImeSelect(himc, true)                             // activate IME on context
  → return WinImmSession { hwnd, himc, pending_commit: None }

WinImmSession::destroy()
  → ImeSelect(himc, false)
  → ImmDestroyContext(himc)
  → DestroyWindow(hwnd)
```

---

## Rime Plugin Subsystem Details

### Plugin Registration (`remote_module.cc`)

```cpp
// Registers three components under the "grpc_proxy_v2" module
RIME_REGISTER_MODULE(remote) {
    Registry& r = Registry::instance();
    r.Register("remote_processor", new Component<RemoteProcessor>);
    r.Register("remote_segmentor", new Component<RemoteSegmentor>);
    r.Register("remote_translator", new Component<RemoteTranslator>);
}
```

### Processing Chain

```
Engine receives key event
  → ascii_composer (Shift toggle, handled locally by Rime)
  → RemoteProcessor::ProcessKeyEvent(key_event)
      if key is Shift-only release → handle ASCII toggle locally
      else → client->ProcessKey(session, keycode, mask)
        if accepted → client->GetContext(session, &cached_context)
                     → check for commit via client->GetCommit()
                     → return kAccepted
        if rejected → return kNoop (pass through to application)

Engine processes segments
  → RemoteSegmentor::Proceed(segmentation)
      tag entire input as "remote"

Engine translates
  → RemoteTranslator::Query(input, segment)
      read cached_context from SharedState
      for each candidate in context.menu.candidates:
        create SimpleCandidate("remote", ...)
        assign select_key based on position (1-5 or a-e for v-mode)
      return Translation
```

### v-mode Detection

When the preedit matches the regex `^v\d` (configurable), the plugin switches from numeric select keys (`1-5`) to alphabetic (`a-e`). This accommodates Sogou's v-mode (expression input) where digits are part of the input.

### ASCII Mode Toggle

`RemoteProcessor` handles Shift-based ASCII mode switching locally (no gRPC call). Three commit styles on toggle:
- `commit_text` — commit current composition text
- `commit_code` — commit raw input code
- `noop` — discard composition

The ASCII mode state is tracked in `SharedState` and used by `RemoteProcessor` to short-circuit key events when in ASCII mode.

---

## Known Pitfalls and Gotchas

### Wine Environment
- Wine config update dialog triggers on every `wine` command if not suppressed. Set `WINEDLLOVERRIDES="mscoree=d;mshtml=d"` and use `WINEDEBUG=-all`.
- Use 32-bit prefix for 32-bit IMEs: `WINEARCH=win32 WINEPREFIX=~/.win32 winecfg`.
- Use 64-bit prefix for 64-bit IMEs (Sogou 16.3): `WINEPREFIX=~/.wine64 winecfg`.
- **Xvfb is only required for IME installer execution** (e.g., `xvfb-run -a wine sogou_pinyin_105b_xp.exe`). The host process itself does NOT need a display — it uses `HWND_MESSAGE` (message-only windows) that work without a display server.

### IME DLL Quirks
- **Sogou 10.5b (32-bit)**: Requires gate byte at offset `+0x36c` to be set for `ImeProcessKey` to accept keys. Official installer sets this automatically; manually extracted DLL does not.
- **Sogou 16.3 (64-bit)**: `byte_3554` flag controls Win7 vs Win8+ code path. Win8+ path uses custom `SendMessageW(0x8BB8)` that bypasses COMPOSITIONSTRING. Fix: patch `kernel32.dll` PE version to 6.1.7601.
- **QQPinyin 6.6 (32-bit)**: Spawns `UserCenter.exe` IPC process. If it fails, IME still works but logs errors. Most cooperative IMM behavior.
- Some IMEs return empty `GCS_RESULTSTR` for standalone punctuation — use the fallback table.
- `ImeToAsciiEx` may return `WM_CHAR` messages in `trans_msgs` that need to be processed.
- 64-bit TRANSMSG is 24 bytes (u32+pad+u64+u64); 32-bit is 12 bytes. Use platform-correct struct size.

### Encoding
- All IMM string queries must use wide (UTF-16) versions: `ImmGetCompositionStringW`.
- WINABC in Wine returns GBK-encoded bytes in "Unicode" positions — requires codepage fallback decoding.
- **Sogou PE version detection**: reads `max(kernel32.dll PE FileVersion, RtlGetNtVersionNumbers)`. Wine's kernel32 reports 10.0 → triggers Win8+ code path (`byte_3554=1`) → COMPOSITIONSTRING bypassed. Fix: patch PE version to `6.1.7601` (Win7 SP1).

### Thread Affinity
- Win32 IME calls must happen on the thread that owns the HWND. The `ChannelImmAdapter` isolates all IMM calls to a dedicated thread via `tokio::sync::oneshot` channels.
- Creating/destroying windows from the tokio runtime thread will deadlock.

### Latency
- Synchronous `RimeBackend` trait methods starved the tokio reactor in early versions. Solution: all trait methods are `async fn` with `#[tonic::async_trait]`.
- Achieved p50 ~2.5ms, p95 ~3.7ms, p99 ~6.6ms per keystroke in v1 benchmarks.

### gRPC Lifecycle
- Session must be opened before any `ProcessKey` call. Session IDs are opaque strings (typically UUIDs).
- During DLL unload (C++ side), gRPC threads are already dead — `GrpcImeClientV2::~GrpcImeClientV2()` must NOT make RPC calls. Use `Shutdown()` method before unload.

### Production Deployment
- The 64-bit host runs as a systemd service (`ime64.service`) on Debian 13 (trixie) with Wine staging.
- Dedicated `ime` user (uid 1001). Sandboxed via `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`.
- Sogou 16.3 installed from a community "green" (portable) package — no official installer needed, no Xvfb required.
- Sogou telemetry blocked via iptables: all outbound from uid 1001 rejected except `127.0.0.1`. This prevents cloud candidate fetches, update checks, and usage reporting.
- `DISPLAY=""` explicitly — host uses `HWND_MESSAGE`, no display server needed.
- See `SETUP_AND_USAGE.md § Production Deployment` for full details.

---

## File Index

```
src/grpc-contract-v2/
  proto/rime_service.proto          # Canonical v2 protobuf definition

src/ime-grpc-host-v2/
  Cargo.toml                        # Rust package manifest
  build.rs                          # Proto compilation (tonic-build)
  src/main.rs                       # CLI entry: --bind, --ime-path
  src/lib.rs                        # Module exports
  src/server.rs                     # RimeServerImpl (6 RPC handlers)
  src/client.rs                     # Demo gRPC client
  src/test_client.rs                # Minimal test client
  src/backend/
    mod.rs                          # RimeBackend async trait
    native.rs                       # Linux Rime FFI backend
    rime_ffi.rs                     # C FFI bindings for librime
  src/win_imm/
    mod.rs                          # ImmRimeAdapter (process_vk, get_context, get_commit)
    imm_ops.rs                      # IME DLL loading, composition/candidate queries
    session.rs                      # WinImmSession (HWND + HIMC lifecycle)
    vk_map.rs                       # Rime keysym → Win32 VK mapping
    punct_map.rs                    # Punctuation fallback table
    channel_adapter.rs              # Thread-isolated async adapter
    keys.rs                         # VK injection stub
    thread_pump.rs                  # Message pump stub
  src/bin/
    test-delayed-window.rs          # Window creation test
    test-notify.rs                  # IMM notification test
    test-thread-window.rs           # Background thread window test
    test-tokio-window.rs            # Async window creation test
  tests/
    test_imm.rs                     # Win32 IMM integration tests
    test_grpc_punctuation.rs        # gRPC punctuation round-trip
    test_version_detect.rs          # PE version detection test

src/rime-remote/                          # v3: proper Rime pipeline components
  CMakeLists.txt                    # Dual-mode build (standalone / librime-integrated)
  remote.schema.yaml                # Schema config (backend_address, v_mode_regex)
  src/
    remote_module.cc                # RIME_REGISTER_MODULE(remote)
    remote_processor.h/cc           # Key event → gRPC forwarding, ASCII toggle
    remote_segmentor.h/cc           # Tags all input as "remote"
    remote_translator.h/cc          # Converts gRPC candidates to Rime candidates
    shared_state.h                  # RemoteSharedState + RemoteStateRegistry

src/rime-grpc-proxy-v2/                   # v2: Rime C API hooking (superseded by rime-remote)
  CMakeLists.txt                    # Build system (same dual-mode as rime-remote)
  grpc_proxy.schema.yaml            # Schema config (grpc_proxy/ prefix)
  src/
    grpc_proxy_module.cc            # Monolithic plugin: API hooks + state management
    grpc_client.cc/h                # GrpcImeClientV2 (shared with rime-remote)
    grpc_key_event_processor.cc/h   # Stub processor (returns kNoop for all keys)
    codepage.h                      # Encoding utilities
```

### Version Lineage

| Version | Component | Architecture | Key Limitation |
|---|---|---|---|
| v1 | `rime-grpc-proxy` + `ime-grpc-host` | Custom proto, worker pool | Monolithic 70KB main.rs, v1 proto |
| v2 | `rime-grpc-proxy-v2` + `ime-grpc-host-v2` | Rime C API hooking | GrpcKeyEventProcessor returns kNoop for all keys; no ascii_composer; stdbool ABI workaround needed for Squirrel |
| v3 | `rime-remote` + `ime-grpc-host-v2` | Standard Rime pipeline | Production. Proper Processor/Segmentor/Translator. ASCII mode handled locally. |

### Supported IME DLLs

| IME | Version | Arch | Wine Prefix | Notes |
|---|---|---|---|---|
| QQPinyin | 6.6.6304 | 32-bit | `.wine32` | Primary target, most cooperative IMM behavior |
| SogouPY (Sogou 10.5b) | 10.5.0.4737 | 32-bit | `.wine32` | XP edition; +0x36c gate byte; PE version patching needed |
| SogouPY (Sogou 16.3) | 16.3.0.3318 | 64-bit | `.wine64` | PE32+ x64; byte_3554 version gate; 24-byte TRANSMSG; same DLL name as 10.5b; installed from green package via `a.bat`; telemetry blocked by iptables |
