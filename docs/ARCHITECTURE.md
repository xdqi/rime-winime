# System Architecture

> Current-state architecture of the Sogou IME gRPC Bridge.
> No historical context — see `DEVELOPMENT_HISTORY.md` for the journey.

---

## Overview

The system runs closed-source Windows IME engines on any platform by combining three technologies:

1. **Wine** (Linux/macOS) or **native Windows** — executes the Windows IME DLL
2. **gRPC** — bridges Rime input framework to the IME host over TCP
3. **librime plugin** — integrates into Rime frontends: **Weasel** (Windows), **Squirrel** (macOS), or **fcitx5-rime / ibus-rime** (Linux)

The system runs across two processes connected by gRPC. The plugin side (rime-remote, v3) is a clean refactor of the earlier rime-grpc-proxy-v2 (v2), which used Rime C API hooking. The host side (ime-grpc-host-v2) supports both 32-bit and 64-bit IME DLLs:

| IME DLL | Architecture | Host Target | Wine Prefix |
|---|---|---|---|
| QQPinyin 6.6 | 32-bit | `i686-pc-windows-gnu` | `.wine32` |
| Sogou 10.5b (XP edition) | 32-bit | `i686-pc-windows-gnu` | `.wine32` |
| Sogou 16.3 | 64-bit (PE32+) | `x86_64-pc-windows-gnu` | `.wine64` |

---

## Layered Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                         User Application                             │
│                    (terminal, editor, browser)                        │
└─────────────────────────────┬────────────────────────────────────────┘
                              │ XIM / IBus / Fcitx5 protocol
┌─────────────────────────────▼────────────────────────────────────────┐
│                     Rime Frontend Framework                           │
│                  (ibus-rime / fcitx5-rime / etc.)                     │
└─────────────────────────────┬────────────────────────────────────────┘
                              │ librime C API
┌─────────────────────────────▼────────────────────────────────────────┐
│                        librime Engine                                 │
│  ┌─────────────┐  ┌──────────────────┐  ┌──────────────────────┐     │
│  │ ascii_       │  │ RemoteProcessor  │  │ RemoteSegmentor      │     │
│  │ composer     │  │ (key dispatch)   │  │ (tag = "remote")     │     │
│  └─────────────┘  └──────┬───────────┘  └──────────────────────┘     │
│                          │                                            │
│  ┌───────────────────────▼──────────────────────────────────────┐    │
│  │                  SharedState (per engine)                      │    │
│  │  - GrpcImeClientV2 (gRPC stub, singleton per address)        │    │
│  │  - remote_session_id                                          │    │
│  │  - cached RimeContextProto                                    │    │
│  │  - v_mode_regex, pending_commit                               │    │
│  └───────────────────────┬──────────────────────────────────────┘    │
│                          │                                            │
│  ┌───────────────────────▼──────────────────────────────────────┐    │
│  │               RemoteTranslator                                │    │
│  │  reads cached_context → generates Rime candidates             │    │
│  └──────────────────────────────────────────────────────────────┘    │
│                      rime-remote plugin                               │
└─────────────────────────────┬────────────────────────────────────────┘
                              │ gRPC (rime.service.v2)
                              │ TCP :50051
┌─────────────────────────────▼────────────────────────────────────────┐
│                   ime-grpc-host-v2 (Rust process)                     │
│  ┌──────────────────────────────────────────────────────────────────┐│
│  │                     RimeServerImpl                                ││
│  │               (tonic gRPC service handler)                        ││
│  │                                                                   ││
│  │  dispatch to:                                                     ││
│  │  ┌────────────────────┐     ┌──────────────────────────────────┐ ││
│  │  │  NativeRimeBackend │     │          ImmRimeAdapter          │ ││
│  │  │  (Linux only)      │     │  ┌────────────────────────────┐  │ ││
│  │  │  Links librime     │ OR  │  │    ChannelImmAdapter       │  │ ││
│  │  │  C FFI             │     │  │    (async ↔ sync bridge)   │  │ ││
│  │  └────────────────────┘     │  └──────────┬─────────────────┘  │ ││
│  │                             │             │ channel             │ ││
│  │                             │  ┌──────────▼─────────────────┐  │ ││
│  │                             │  │  Dedicated Win32 Thread    │  │ ││
│  │                             │  │  - WinImmSession (HWND,    │  │ ││
│  │                             │  │    HIMC per session)        │  │ ││
│  │                             │  │  - ImeProcessKey()          │  │ ││
│  │                             │  │  - ImeToAsciiEx()           │  │ ││
│  │                             │  │  - ImmGetCompositionString()│  │ ││
│  │                             │  │  - ImmGetCandidateList()    │  │ ││
│  │                             │  └──────────┬─────────────────┘  │ ││
│  │                             │             │ Win32 IMM DDI       │ ││
│  │                             └─────────────┼────────────────────┘ ││
│  └───────────────────────────────────────────┼──────────────────────┘│
│                                              │                       │
│  ┌───────────────────────────────────────────▼──────────────────────┐│
│  │                    IME DLL (under Wine)                           ││
│  │           QQPinyin.ime / SogouPY.ime / WINABC.ime                ││
│  └──────────────────────────────────────────────────────────────────┘│
│                   Wine / Windows 32-bit process                       │
└──────────────────────────────────────────────────────────────────────┘
```

---

## Component Responsibilities

### rime-remote (v3, C++ librime plugin)

| Component | Role |
|---|---|
| `RemoteProcessor` | Intercepts key events from the Rime engine. Forwards them via gRPC `ProcessKey()`. On accept, fetches context/commit. Handles ASCII mode toggle locally (no RPC). |
| `RemoteSegmentor` | Tags the entire input as a single `"remote"` segment so the default Rime segmentors do not interfere. |
| `RemoteTranslator` | Reads the cached `RimeContextProto` from `RemoteSharedState` and converts each candidate into a Rime `SimpleCandidate` with appropriate select keys. |
| `RemoteSharedState` | Per-engine state. Caches the gRPC client stub, session ID, context, and runtime flags. All three components coordinate through this via `RemoteStateRegistry`. |
| `GrpcImeClientV2` | Thread-safe gRPC client stub. One instance shared across all engines targeting the same backend address. Created once, destroyed at module unload. |

**Predecessor note:** `rime-grpc-proxy-v2` (v2) used Rime C API hooking — a single `grpc_proxy_module.cc` that intercepted `process_key`, `get_context`, `get_commit` etc. at the function-pointer level. This had fundamental limitations: `GrpcKeyEventProcessor` returned `kNoop` for all keys, ascii_composer never ran, and a stdbool ABI workaround was needed for Squirrel (macOS). `rime-remote` (v3) replaces this with proper Rime pipeline components.

### ime-grpc-host-v2 (Rust gRPC server)

| Component | Role |
|---|---|
| `RimeServerImpl` | tonic gRPC service implementation. Receives RPC calls, acquires backend mutex, delegates to the active `RimeBackend` implementation. |
| `RimeBackend` (trait) | Async interface with 6 methods matching the 6 RPC calls. Two implementations: `NativeRimeBackend` (Linux) and `ImmRimeAdapter` (Windows/Wine). |
| `NativeRimeBackend` | Linux-only. Loads librime via C FFI, creates sessions via `RimeCreateSession()`, processes keys, queries context. Used for testing without Wine. |
| `ImmRimeAdapter` | Windows/Wine. Loads a `.ime` DLL and manages sessions as HWND+HIMC pairs. Converts Rime keysyms to Win32 VK codes, calls IME DDI functions, parses composition/candidate structures. |
| `ChannelImmAdapter` | Wraps `ImmRimeAdapter` to enforce thread affinity. All Win32 calls are dispatched to a dedicated thread via `tokio::sync::oneshot` channels. |
| `WinImmSession` | Represents an active IME session. Each session owns a hidden message-only window (HWND) and an input method context (HIMC). |
| `ImeFunctions` | Struct of function pointers loaded from the IME DLL: `ImeProcessKey`, `ImeToAsciiEx`, `ImeSelect`, `NotifyIME`, etc. |

---

## Data Flow

### Keystroke Processing (Happy Path)

```
User types 'n' → Application → Rime Frontend → librime engine
  → RemoteProcessor::ProcessKeyEvent(key='n', mask=0)
    → GrpcImeClientV2::ProcessKey(session_id, keycode=0x6e, mask=0)
      → [gRPC] → RimeServerImpl::process_key()
        → ImmRimeAdapter::process_key(session_id, 0x6e, 0)
          → rime_to_vk(0x6e) → VK_N, not shifted
          → ImeProcessKey(himc, VK_N, lParam) → TRUE (accepted)
          → ImeToAsciiEx(VK_N, ...) → [WM_IME_COMPOSITION]
      ← returns ProcessKeyResponse { accepted: true }
    → GrpcImeClientV2::GetContext(session_id)
      → [gRPC] → RimeServerImpl::get_context()
        → ImmRimeAdapter::get_context(session_id)
          → ImmGetCompositionString(himc, GCS_COMPSTR) → "n"
          → ImmGetCandidateList(himc, 0) → ["你", "那", "年", ...]
      ← returns GetContextResponse { context: { composition: { preedit: "n" }, menu: { candidates: [...] } } }
    → cache context in SharedState
    ← return kAccepted

Engine proceeds to segmentation
  → RemoteSegmentor::Proceed() → tags input as "remote"

Engine proceeds to translation
  → RemoteTranslator::Query(segment "remote")
    → reads SharedState.cached_context
    → creates SimpleCandidate for each candidate: "你", "那", "年", ...
    ← returns Translation with all candidates

Rime Frontend displays candidates to user
```

### Candidate Selection

```
User presses '1' to select first candidate
  → RemoteProcessor::ProcessKeyEvent(key='1')
    → GrpcImeClientV2::SelectCandidate(session_id, index=0)
      → [gRPC] → RimeServerImpl::select_candidate()
        → ImmRimeAdapter::select_candidate(session_id, 0)
          → NotifyIME(himc, NI_SELECTCANDIDATESTR, ...)
      ← returns SelectCandidateResponse { success: true }
    → GrpcImeClientV2::GetCommit(session_id)
      ← returns "你"
    → engine->CommitText("你")
    ← return kAccepted
```

### Session Lifecycle

```
Rime engine creates new session (e.g., user opens input in app)
  → RemoteProcessor::Initialize(engine)
    → GrpcImeClientV2::OpenSession(rime_session_id, schema_id)
      → [gRPC] → RimeServerImpl::open_session()
        → ImmRimeAdapter::open_session()
          → CreateWindowExW(hidden message window)
          → ImeInquire() + ImeSelect(himc, true)
      ← returns session_id (UUID)
    → store session_id in SharedState

Rime engine destroys session
  → RemoteProcessor::~RemoteProcessor()
    → GrpcImeClientV2::DestroySession(session_id)
      → [gRPC] → RimeServerImpl::destroy_session()
        → ImmRimeAdapter::destroy_session(session_id)
          → ImeSelect(himc, false)
          → ImmDestroyContext(himc)
          → DestroyWindow(hwnd)
```

---

## Thread Model

### rime-remote (C++ side)

Runs in the main librime thread. All gRPC calls are **synchronous** (blocking) with a configurable timeout (`rpc_timeout_ms`, default 200ms). This is acceptable because librime's key processing is inherently synchronous.

### ime-grpc-host-v2 (Rust side)

```
┌─────────────────────────────────────────────────┐
│              tokio async runtime                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐      │
│  │ gRPC     │  │ gRPC     │  │ gRPC     │      │
│  │ handler  │  │ handler  │  │ handler  │ ...  │
│  │ task     │  │ task     │  │ task     │      │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘      │
│       │              │              │             │
│       │    ChannelImmAdapter        │             │
│       │    (oneshot channels)       │             │
│       │              │              │             │
└───────┼──────────────┼──────────────┼─────────────┘
        │              │              │
        ▼              ▼              ▼
┌─────────────────────────────────────────────────┐
│          Dedicated Win32 Thread                  │
│  - Owns all HWNDs and HIMCs                     │
│  - Runs Win32 message pump                      │
│  - Processes all IMM DDI calls sequentially      │
│  - Responds via oneshot channel send             │
└─────────────────────────────────────────────────┘
```

- **Why a dedicated thread?** Win32 IMM functions are thread-affine. The HWND must be owned by the calling thread, and message dispatch (DispatchMessage) only works on the owning thread.
- **Why channels?** Tokio's async runtime uses a thread pool. We cannot guarantee which thread a gRPC handler runs on. The `ChannelImmAdapter` serializes all Win32 calls through a single thread.

---

## Configuration

### rime-remote

**`remote.schema.yaml`** (placed in Rime user directory):

```yaml
schema:
  schema_id: remote
  name: "Remote IME"

engine:
  processors:
    - ascii_composer
    - remote_processor
  segmentors:
    - remote_segmentor
  translators:
    - remote_translator

remote_processor:
  backend_address: "127.0.0.1:50051"
  rpc_timeout_ms: 200
  v_mode_preedit_regex: "^v\\d"
  ascii_composer:
    switch_key:
      Shift_L: commit_text    # Left Shift → commit and toggle ASCII
      Shift_R: noop            # Right Shift → discard and toggle ASCII
```

### ime-grpc-host-v2

**CLI arguments:**

```
ime-grpc-host-v2 --bind 127.0.0.1:50051 --ime-path "C:\windows\system32\QQPinyin.ime"
```

| Flag | Default | Description |
|---|---|---|
| `--bind` | `127.0.0.1:50051` | gRPC listen address |
| `--ime-path` | (required) | Path to Windows `.ime` DLL |

**Environment variables (Wine):**

```bash
export WINEPREFIX=~/.win32
export WINEARCH=win32
export WINEDEBUG=-all
export WINEDLLOVERRIDES="mscoree=d;mshtml=d"
export DISPLAY=:99                # Xvfb virtual display
```

---

## Deployment Topology

```
┌──────────────── Linux Host ─────────────────────┐
│                                                  │
│  ┌──────────────┐     ┌──────────────────────┐  │
│  │ Rime Frontend│     │ Xvfb :99             │  │
│  │ (fcitx5-rime)│     │ (virtual display)    │  │
│  └──────┬───────┘     └──────────────────────┘  │
│         │                                        │
│  ┌──────▼───────┐     ┌──────────────────────┐  │
│  │ librime      │     │ Wine wrapper script  │  │
│  │ + rime-remote│────►│ wine ime-grpc-host-  │  │
│  │   plugin     │:5005│ v2.exe               │  │
│  └──────────────┘  1  │ --bind 127.0.0.1:50051 │  │
│                       │ --ime-path QQPinyin  │  │
│                       └──────────────────────┘  │
│                                                  │
└──────────────────────────────────────────────────┘
```

Both processes run on the same machine. The gRPC channel uses TCP on localhost. The Wine process is headless (Xvfb provides a virtual X display for Wine's internal needs).
