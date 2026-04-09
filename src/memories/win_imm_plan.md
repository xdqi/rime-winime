# Windows IMM Host Phase 1 Implementation Plan

## Architecture Overview
- **Single Process, Multi-Session:** The Rust Host will run as a single process, maintaining a `HashMap<usize, WinImmSession>`. Each session corresponds to a private Windows `HIMC` (Input Method Context) and a hidden `HWND` (Message-Only Window).
- **Target Architecture:** Strictly 32-bit (`i686-pc-windows-gnu`). This ensures absolute ABI compatibility with 32-bit IME DLLs (e.g., QQPinyin).
- **Invocation Strategy:** Active Invocation (Direct Export Calling). We will use `LoadLibrary` to load the 32-bit `.ime` DLL as a standard dynamic library and manually invoke its IMM DDI exports (`ImeInquire`, `ImeSelect`, `ImeProcessKey`, `ImeToAsciiEx`). No system-level IME installation (`LoadKeyboardLayout`) or Windows message hooking will be used in Phase 1, as this direct-call approach is proven to work with QQPinyin.

## Implementation Steps

### 1. Session & State Management (`src/win_imm/session.rs`)
- Create a `WinImmSession` struct to hold `session_id`, `hwnd` (hidden window), and `himc` (input context).
- Update `ImmRimeAdapter` in `mod.rs` to manage a thread-safe `HashMap<usize, WinImmSession>`.

### 2. Global Initialization (DLL Loading in `src/win_imm/imm_ops.rs`)
- Load the 32-bit IME DLL using `libloading` (or `windows` crate module APIs).
- Map the exported functions:
  - `ImeInquire`
  - `ImeSelect`
  - `ImeProcessKey`
  - `ImeToAsciiEx`
- Call `ImeInquire` globally exactly once to initialize the IME engine's internal data structures.

### 3. Session Lifecycle (`open_session` & `destroy_session`)
- **Open Session**:
  - Dynamically create a lightweight hidden window (`HWND_MESSAGE`) via `CreateWindowExW`.
  - Allocate a true input context via `ImmCreateContext()`.
  - Bind the context to the window via `ImmAssociateContextEx()`.
  - Activate the IME for this context via `ImeSelect(himc, TRUE)`.
- **Destroy Session**:
  - Deactivate via `ImeSelect(himc, FALSE)`.
  - Cleanup resources using `ImmDestroyContext()` and `DestroyWindow()`.

### 4. Input Routing (`process_key`)
- Look up the `HIMC` via `session_id`.
- Construct a 256-byte `KEYSTATE` array (`lpbKeyState`) based on `KeyEvent` modifiers (Shift, Ctrl, etc.).
- Call `ImeProcessKey`. If it returns `TRUE` (consumed by IME):
  - Call `ImeToAsciiEx` to let the IME engine process the keystroke and update the internal `HIMC` state natively.

### 5. State Synchronization (`get_context` & `get_commit`)
- **Get Context**:
  - Query preedit/composition strings using `ImmGetCompositionStringW(himc, GCS_COMPSTR)`.
  - Query candidate lists using `ImmGetCandidateListW`.
  - Map IMM structs to `RimeContextProto` (including pagination).
- **Get Commit**:
  - Query finalized text using `ImmGetCompositionStringW(himc, GCS_RESULTSTR)`.
  - Return the committed text (and optionally execute `ImmSetCompositionString(himc, SCS_SETSTR)` to clear the buffer if the IME does not auto-clear).

## Design Decisions & Constraints (Confirmed with User)
1. **Candidate List Fetching**: Trust the IMM DDI (`ImmGetCandidateListW`). Standard Windows IMEs implement this properly to support app-rendered UI (e.g., in games, conime). 
2. **Modifier Key States**: Start with a simple 256-byte `KEYSTATE` mapping (toggle flags for Shift/Ctrl/Alt). Full state simulation will be reserved as a future enhancement if edge cases like Caps Lock or long-press cause issues.
3. **Background Threads & Networking**: The IME should remain as stateless and inactive as possible. No dedicated true message pump / background thread will be artificially maintained for the IME unless strictly necessary for functionality. We explicitly do *not* want the IME to connect to the internet or keep long-running tasks alive.
4. **Context Synchronization**: Synchronize as many fields as physically possible between RIME (`RimeContextProto`) and IMM (like `GCS_CURSORPOS`, `length`, `selection`). If an IMM concept fundamentally cannot project onto RIME's proto cleanly, we will stop and escalate for manual review.
5. **Concurrency & Thread Affinity**: Originally, we hoped to ignore thread affinity. However, when integrated into a Tokio gRPC server, `ImeProcessKey` runs on an arbitrary `tokio` thread, which causes a classic Win32 deadlock when the IME invokes `SendMessageW` targeting a hidden window (`HWND`) created on a different thread that isn't pumping messages.
   - **Resolution**: Use a `channel_adapter` (MPSC pattern) to isolate all `RimeBackend` calls (Context lifecycle, `ImeProcessKey`, Result fetching) onto a **single, dedicated OS thread**. This ensures the HWND and IME instances strictly share the same native thread thread block, enabling direct `WndProc` dispatches without requiring a manual `GetMessage` loop.

## 6. Real-world Behavior Insights (QQPinyin testing)
Through direct testing with QQPinyin via `ImeProcessKey` and `ImeToAsciiEx`, the following findings validate the minimal integration approach:
- **Partial Commits:** No manual host-side buffer mixing is needed. If a user types `nihaoshijie` and selects candidate `2.你好` (`VK_2`), the IME automatically consumes it. The preedit (`GCS_COMPSTR`) immediately updates smoothly to a visual hybrid string like `你好shi'jie`, and the Candidate list recalculates to matching items like `世界`. The final commit happens atomically later.
- **Capitalization Modifiers:** Passing Shift state in the `KEYSTATE` block (for example, `[VK_SHIFT] = 0x80`) correctly triggers modes like English-capitalized insertion natively (`U` followed by `pan` yields candidates like `U盘`).
- **Unconsumed Keys (Punctuation/Numbers):** When no composition is active, pressing numeric keys (`VK_1`, `VK_2`) yields `FALSE` from `ImeProcessKey`. The Host appropriately detects this and treats it as raw unconsumed input ("1" directly passes through to the client context). Chinese punctuation keys (like `VK_OEM_COMMA`) are consumed (`TRUE`) and successfully yield localized output like `，` in the `RimeContextProto`'s commit result immediately.