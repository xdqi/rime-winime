# gRPC v2 Protocol Reference

> Complete API reference for the `rime.service.v2` gRPC protocol.

---

## Overview

The `rime.service.v2` protocol defines a stateful session-based API for remote input method processing. A client (rime-remote plugin) opens a session, sends key events, queries composition state and committed text, and eventually destroys the session. All RPCs are unary (request-response).

**Proto file:** `src/grpc-contract-v2/proto/rime_service.proto`
**Package:** `rime.service.v2`
**Default port:** `50051`

---

## Service Definition

```protobuf
service RimeService {
  rpc OpenSession(OpenSessionRequest) returns (OpenSessionResponse);
  rpc ProcessKey(ProcessKeyRequest) returns (ProcessKeyResponse);
  rpc GetContext(GetContextRequest) returns (GetContextResponse);
  rpc GetCommit(GetCommitRequest) returns (GetCommitResponse);
  rpc DestroySession(DestroySessionRequest) returns (DestroySessionResponse);
  rpc SelectCandidateOnCurrentPage(SelectCandidateRequest) returns (SelectCandidateResponse);
}
```

---

## RPC Methods

### OpenSession

Creates a new IME session on the server. Must be called before any other session-scoped RPC.

| Field | Type | Description |
|---|---|---|
| **Request** | | |
| `schema_id` | `string` | Rime schema ID (e.g., `"remote"`). Informational; the backend IME decides its own schema. |
| **Response** | | |
| `session_id` | `string` | Opaque session identifier (UUID). Used in all subsequent RPCs. |

**Server behavior:**
- Creates a hidden Win32 window (HWND) and input method context (HIMC)
- Activates the IME via `ImeSelect(himc, true)`
- Returns a stable UUID for the session lifetime

---

### ProcessKey

Sends a key event to the IME for processing.

| Field | Type | Description |
|---|---|---|
| **Request** | | |
| `session_id` | `string` | Session ID from `OpenSession`. |
| `key_event` | `KeyEvent` | Key event with keycode and modifier mask. |
| **Response** | | |
| `session_id` | `string` | Echo of the session ID. |
| `accepted` | `bool` | `true` if the IME consumed the key; `false` if the key should be passed through. |

**Key event encoding:**
- `keycode` uses Rime/XKB keysym values (NOT Win32 VK codes)
  - Lowercase letters: `0x61`–`0x7a` (a–z)
  - Uppercase letters: `0x41`–`0x5a` (A–Z)
  - Digits: `0x30`–`0x39` (0–9)
  - Return: `0xff0d`
  - BackSpace: `0xff08`
  - Escape: `0xff1b`
  - Space: `0x20`
- `modifier` is a bitmask:
  - Shift: `1 << 0` (0x1)
  - Lock (Caps): `1 << 1` (0x2)
  - Control: `1 << 2` (0x4)
  - Alt: `1 << 3` (0x8)
  - Release flag: `1 << 30` (for key-up events)

**Server behavior:**
- Converts Rime keysym → Win32 VK (via `rime_to_vk`)
- Calls `ImeProcessKey()` then `ImeToAsciiEx()` if accepted
- Pumps resulting IME messages
- Caches composition/result strings internally (retrieved via `GetContext`/`GetCommit`)

---

### GetContext

Retrieves the current composition state and candidate list.

| Field | Type | Description |
|---|---|---|
| **Request** | | |
| `session_id` | `string` | Session ID. |
| **Response** | | |
| `session_id` | `string` | Echo. |
| `context` | `RimeContextProto` | Current composition and menu state. May be empty if no active composition. |

**Typical call pattern:** Called immediately after a `ProcessKey` that returned `accepted: true`.

---

### GetCommit

Retrieves committed text (if any) from the last key processing cycle.

| Field | Type | Description |
|---|---|---|
| **Request** | | |
| `session_id` | `string` | Session ID. |
| **Response** | | |
| `session_id` | `string` | Echo. |
| `commit_text` | `string` | The committed text (UTF-8). Empty if no commit. |
| `has_commit` | `bool` | `true` if text was committed in the last cycle. |

**Server behavior:**
- Returns `pending_commit` from the session, then clears it
- Commit text is produced when: user selects a candidate, presses Space on a completed composition, or the IME auto-commits

---

### SelectCandidateOnCurrentPage

Selects a candidate by index on the current page.

| Field | Type | Description |
|---|---|---|
| **Request** | | |
| `session_id` | `string` | Session ID. |
| `index` | `int32` | 0-based candidate index on the current page. |
| **Response** | | |
| `success` | `bool` | `true` if selection succeeded. |

**Server behavior:**
- Calls `NotifyIME(himc, NI_SELECTCANDIDATESTR, ...)` or equivalent mechanism
- After selection, the IME may either commit text or update the composition (for multi-stage input)
- Client should call `GetCommit()` after a successful selection to check for committed text

---

### DestroySession

Tears down an IME session and frees associated resources.

| Field | Type | Description |
|---|---|---|
| **Request** | | |
| `session_id` | `string` | Session ID. |
| **Response** | | |
| `success` | `bool` | `true` if the session was found and destroyed. |

**Server behavior:**
- Calls `ImeSelect(himc, false)` to deactivate the IME
- Destroys the HIMC and its associated hidden HWND
- Removes the session from the internal session map

---

## Message Types

### KeyEvent

```protobuf
message KeyEvent {
  uint32 keycode = 1;   // Rime/XKB keysym
  uint32 modifier = 2;  // Modifier bitmask
}
```

### CandidateProto

```protobuf
message CandidateProto {
  string text = 1;      // Candidate text (e.g., "你好")
  string comment = 2;   // Annotation (e.g., pinyin reading)
  double quality = 3;   // Ranking quality score
}
```

### MenuProto

```protobuf
message MenuProto {
  repeated CandidateProto candidates = 1;
  int32 page_size = 2;                    // Max candidates per page
  int32 page_no = 3;                      // Current page (0-based)
  bool is_last_page = 4;                  // No more pages after this
  int32 highlighted_candidate_index = 5;  // Currently highlighted entry
  int32 num_candidates = 6;               // Total candidates across all pages
  string select_keys = 7;                 // Key labels for candidates (e.g., "12345")
}
```

### CompositionProto

```protobuf
message CompositionProto {
  int32 length = 1;      // Total composition length
  int32 cursor_pos = 2;  // Cursor position in preedit
  int32 sel_start = 3;   // Selection start
  int32 sel_end = 4;     // Selection end
  string preedit = 5;    // Preedit text shown to user (e.g., "ni hao")
}
```

### RimeContextProto

```protobuf
message RimeContextProto {
  CompositionProto composition = 1;   // Current preedit state
  MenuProto menu = 2;                 // Current candidate list
  string commit_text_preview = 3;     // What would be committed if Space pressed now
}
```

---

## Typical Interaction Sequence

```
Client                                  Server
  │                                       │
  │  OpenSession(schema_id="remote")      │
  │──────────────────────────────────────►│
  │  OpenSessionResponse(session_id=S)    │
  │◄──────────────────────────────────────│
  │                                       │
  │  ProcessKey(S, 'n', 0)                │
  │──────────────────────────────────────►│
  │  ProcessKeyResponse(accepted=true)    │
  │◄──────────────────────────────────────│
  │                                       │
  │  GetContext(S)                         │
  │──────────────────────────────────────►│
  │  GetContextResponse(preedit="n",      │
  │    candidates=["你","那","年",...])    │
  │◄──────────────────────────────────────│
  │                                       │
  │  ProcessKey(S, 'i', 0)                │
  │──────────────────────────────────────►│
  │  ProcessKeyResponse(accepted=true)    │
  │◄──────────────────────────────────────│
  │                                       │
  │  GetContext(S)                         │
  │──────────────────────────────────────►│
  │  GetContextResponse(preedit="ni",     │
  │    candidates=["你","泥","尼",...])    │
  │◄──────────────────────────────────────│
  │                                       │
  │  ... (more keystrokes) ...            │
  │                                       │
  │  ProcessKey(S, Space, 0)              │
  │──────────────────────────────────────►│
  │  ProcessKeyResponse(accepted=true)    │
  │◄──────────────────────────────────────│
  │                                       │
  │  GetCommit(S)                         │
  │──────────────────────────────────────►│
  │  GetCommitResponse(commit="你好",     │
  │    has_commit=true)                   │
  │◄──────────────────────────────────────│
  │                                       │
  │  DestroySession(S)                    │
  │──────────────────────────────────────►│
  │  DestroySessionResponse(success=true) │
  │◄──────────────────────────────────────│
```

---

## Protocol Design Notes

1. **Stateful sessions** — Each `OpenSession` allocates server-side resources (HWND, HIMC). Clients MUST call `DestroySession` when done. The server has a configurable idle timeout (`--session-timeout-sec`) as a safety net.

2. **Context is pull-based** — The server does not push context updates. The client must call `GetContext` after each `ProcessKey` that returns `accepted: true`.

3. **Commit is separate from context** — Committed text is retrieved via `GetCommit`, not embedded in `GetContext`. This avoids ambiguity when the IME commits AND starts a new composition in the same keystroke.

4. **Keysyms, not VK codes** — The protocol uses Rime/XKB keysyms because the client (rime-remote) runs in a Linux Rime environment. The server is responsible for converting to Win32 VK codes.

5. **One session, one IME context** — Each session maps to exactly one HWND+HIMC pair. Multiple concurrent sessions are supported (e.g., multiple editor windows).

6. **No streaming** — All RPCs are unary. This simplifies error handling and matches the synchronous nature of key event processing.
