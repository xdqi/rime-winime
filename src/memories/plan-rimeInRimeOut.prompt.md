# Plan: Rime-in-Rime-out Architecture (The Ultimate Clean Proxy)

## 1. Architectural Vision
Discard the legacy IMM-centric gRPC protocol that forces Rime to guess the input state. Instead, redefine the gRPC contract around **native librime structures** (`Context`, `Composition`, `Menu`, `CommitText`). 

The C++ proxy (`rime-grpc-proxy`) becomes a dumb, 0-computational terminal that strictly mirrors the remote state. We will implement the backend in two stages: first, a perfect `librime` reference backend (Rime-in-Rime-out) to validate the pipe; second, a Win32 IMM adapter that disguises IMM as a Rime engine.

## 2. Phase 1: Contract Rewrite (`rime_service.proto`)
Completely rewrite the Protobuf contract to match librime API semantics.
- **Service**: `ProcessKey`, `GetContext`, `GetCommit`, `DestroySession`, etc.
- **Messages**:
  - `RimeContextProto` (contains `composition`, `menu`, `candidates`, `commit_text_pending`).
  - `CandidateProto` (`text`, `comment`, `quality`).
  - Drop the bespoke IMM "backend_state_version" guesswork from the payload.

## 3. Phase 2: Rime Reference Implementation (Rust `ime-grpc-host`)
Build a golden standard backend to prove the gRPC tunnel works flawlessly before wrestling with Windows APIs.
- Retain the existing Rust worker pool, prewarming, and isolation logic (crucial for stability).
- Replace `ImeBackend` with a `RimeBackend` trait:
  ```rust
  trait RimeBackend {
      fn process_key(&mut self, key: &KeyEvent) -> bool;
      fn get_context(&self) -> RimeContextProto;
      fn get_commit(&mut self) -> Option<String>;
  }
  ```
- Implement `NativeRimeBackend` in Rust (calling a local Linux/Wine `librime.so`).
- **Validation**: Pass the existing `run_arif_tab_smoke_grpc.sh` tests using this native backend. If it passes, the transport and proxy are 100% bug-free.

## 4. Phase 3: Immerse the IMM Adapter (`win_imm` Refactor)
Once the tunnel is proven, resurrect the Windows IMM logic as an adapter (`ImmRimeAdapter`) that implements the `RimeBackend` trait.
- It will translate gross Win32 messages (`GCS_COMPSTR`, `GCS_RESULTSTR`, `ImmGetCandidateList`) into clean `RimeContextProto` and `CommitText` responses.
- **Code Cleanliness**: The 1500+ line `win_imm.rs` must be dismantled into a `win_imm/` folder:
  - `mod.rs`: Trait implementation and orchestration.
  - `thread_pump.rs`: Win32 hidden window and `GetMessageW` loop.
  - `imm_ops.rs`: Unsafe IMM context (`HIMC`) lifecycle and string extraction (`RESULTSTR`).
  - `keys.rs`: Virtual key injections.

## 5. Phase 4: Purge the C++ Proxy and Schema
Make the Linux/macOS C++ proxy codebase as minimal as possible.
- **Schema (`grpc_proxy.schema.yaml`)**:
  - Delete `speller`, `selector`, `abc_segmentor`, `punctuator`. 
  - Keep exactly ONE processor: `grpc_key_event_processor`.
- **C++ Components**:
  - Delete `grpc_proxy_translator.cc/h` and `grpc_commit_observer.cc/h`.
  - The `grpc_key_event_processor` becomes the Dictator: it sends `ProcessKey`, reads the `RimeContextProto`, brutally overwrites `engine_->context()`, and fires `engine_->context()->CommitText()` if there's a pending commit. Everything is intercepted, yielding `kAccepted`.

## 6. End State
- **C++ Proxy**: ~2 files, completely stateless, no regex, no string manipulation.
- **Rust Host**: Clean modules, resilient worker pool, interchangeable backends (Native Rime vs. Win32 IMM).
- **Correctness**: Multi-round candidate selection and backspacing will "just work" because the sync happens at the Context level, not the keystroke-guess level.
