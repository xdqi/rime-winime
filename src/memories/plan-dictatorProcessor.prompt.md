# Plan: The "Dictator Processor" Architecture (Rime as a Dumb Frontend)

## 1. Goal
Completely remove local input logic (spelling, segmentation, translating, and selecting) from Rime. Delegate 100% of the IME state machine (Composition string, Candidate list, and Commits) to the remote Windows IMM backend (`ime-grpc-host`) via gRPC. 

## 2. Phase 1: Update gRPC Contract (`ime_proxy.proto`)
Modify the Protobuf contract to allow the host to send implicitly/explicitly committed text directly as a response to a keystroke.
- Add `string committed_text = 7;` to `SendKeyEventResponse`.

## 3. Phase 2: Update and Refactor Win_IMM Backend (`ime-grpc-host`)
The monolithic `win_imm.rs` (over 1500 lines) has become a technical debt. Before adding `GCS_RESULTSTR` logic, we will aggressively refactor the Rust host to reflect its true nature: a Windows-centric IMM bridging daemon. This avoids bloating one file with message pumps, FFI, key conversions, and state management.

### Rust (`ime-grpc-host`) Refactoring:
Convert `backend/win_imm.rs` into a structured module directory `backend/win_imm/`:
- **`mod.rs`**: The implementation of `ImeBackend` and the `WinImmBackend` orchestrator.
- **`thread_pump.rs`**: The dedicated Win32 message loop thread (`GetMessageW`, `DispatchMessageW`, window procedures).
- **`imm_ops.rs`**: All the unsafe FFI and safe wrappers for `ImmGetCompositionStringW`, `ImmGetCandidateListW`, `GCS_RESULTSTR` extraction, and HIMC context lifecycle.
- **`keys.rs`**: Virtual key mapping (`VkKeyScanExW`), modifiers handling, and hardware key injection logic.
- *Wait to add `GCS_RESULTSTR` until the file is properly split.*

## 4. Phase 3: Implement the "Dictator" Processor (`rime-grpc-proxy`)
Rewrite `grpc_key_event_processor.cc` to act as an orchestrator, and offload the actual manipulation of Rime internals to a new, dedicated, unit-testable (or at least strictly isolated) helper module.
- **`grpc_state_sync.h/cc` (NEW)**: A stateless utility that takes a `SendKeyEventResponse` and mutates a `rime::Context` object.
  - Clears native Rime segmentations and translations.
  - Constructs a mocked-up `Composition` and a `Menu` (injecting `SimpleCandidate`).
  - Calls `context->CommitText()` if `committed_text` is present.
- **`grpc_key_event_processor.cc`**: Intercepts the keystroke, synchronously calls the `GrpcImeClient::SendKeyEvent`, passes the response to `grpc_state_sync`, and always returns `kAccepted`.

## 5. Phase 4: Schema Purge and Code Cleanup (`grpc_proxy.schema.yaml`)
Strip the Rime YAML configuration down to its absolute minimum and garbage collect Dead Code.
- **YAML Config**: Keep ONLY `grpc_key_event_processor` in `processors`. Remove all `segmentors` and `translators`.
- **C++ Code Cleanup**: Delete `grpc_commit_observer.h/cc` and `grpc_proxy_translator.h/cc` completely. Unregister them in `grpc_proxy_module.cc`. We demand a clean codebase with zero legacy POC logic.
