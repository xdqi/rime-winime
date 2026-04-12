# Win32 IMM Internals

> Technical deep dive into the Win32 Input Method Manager (IMM) subsystem.
> Reference material applicable beyond this project.

---

## Overview

The Win32 IMM (Input Method Manager) is the system layer that connects applications to IME (Input Method Editor) DLLs. An IME DLL implements a set of mandatory DDI (Device Driver Interface) exports. The IMM loads the DLL, creates per-thread or per-window input contexts, and orchestrates the key processing pipeline.

This document covers the IMM interface as implemented in Windows NT/2000/XP (NT5) and as emulated by Wine, which is the runtime environment for this project.

---

## IME DLL Interface

Every IME DLL must export these functions:

### ImeInquire

```c
BOOL ImeInquire(LPIMEINFO lpIMEInfo, LPTSTR lpszUIClass, DWORD dwSystemInfoFlags);
```

Called once when the IME is first loaded. Returns the IME's capabilities and UI class name.

**IMEINFO structure:**
```c
typedef struct tagIMEINFO {
    DWORD dwPrivateDataSize;     // Size of per-context private data
    DWORD fdwProperty;           // Capability flags (IME_PROP_*)
    DWORD fdwConversionCaps;     // Conversion mode capabilities
    DWORD fdwSentenceCaps;       // Sentence mode capabilities
    DWORD fdwUICaps;             // UI capabilities
    DWORD fdwSCSCaps;            // SetCompositionString capabilities
    DWORD fdwSelectCaps;         // Selection inheritance capabilities
} IMEINFO;
```

**SogouPY returns:** `fdwProperty = 0x001e0002`, `fdwConversionCaps = 0x00000488`, UI class = `"SoPY_UI"`.

### ImeSelect

```c
BOOL ImeSelect(HIMC hIMC, BOOL fSelect);
```

Activates (`fSelect = TRUE`) or deactivates (`FALSE`) the IME for a given input context. Must be called before any key processing. On activation, the IME initializes its internal state for the context.

### ImeProcessKey

```c
BOOL ImeProcessKey(HIMC hIMC, UINT uVirKey, LPARAM lParam, CONST LPBYTE lpbKeyState);
```

Tests whether the IME will handle a virtual key event. Returns nonzero if the key is consumed, zero if it should pass through.

**Parameters:**
- `hIMC` — input method context handle
- `uVirKey` — virtual key code (`VK_A`, `VK_RETURN`, etc.)
- `lParam` — key message lParam (contains scan code in bits 16–23, repeat count in 0–15, extended flag in bit 24, transition state in bit 31)
- `lpbKeyState` — 256-byte array of key states (bit 7 = pressed, bit 0 = toggled)

**Critical:** `lParam` must contain a valid scan code. If `lParam == 0`, most IMEs reject the key. Use `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)` to generate the scan code.

### ImeToAsciiEx

```c
UINT ImeToAsciiEx(
    UINT uVirKey,
    UINT uScanCode,
    CONST LPBYTE lpbKeyState,
    LPTRANSMSGLIST lpTransMsgList,
    UINT fuState,
    HIMC hIMC
);
```

Processes a key event that was accepted by `ImeProcessKey`. Generates translation messages (TRANSMSG) that describe the IME's output: composition updates, candidate list changes, and committed text.

**Returns:** Number of TRANSMSG entries written to `lpTransMsgList`. If the return value exceeds the list capacity, the IME stores overflow messages in the `hMsgBuf` member of the input context.

**TRANSMSGLIST structure:**
```c
typedef struct tagTRANSMSGLIST {
    UINT     uMsgCount;         // Capacity (input) / Count written (output)
    TRANSMSG TransMsg[1];       // Variable-length array
} TRANSMSGLIST;

typedef struct tagTRANSMSG {
    UINT    message;            // Window message (WM_IME_COMPOSITION, etc.)
    WPARAM  wParam;
    LPARAM  lParam;
} TRANSMSG;
```

**Alignment note:** On 64-bit systems, `TRANSMSG` is 24 bytes (4 + 4 padding + 8 + 8) due to pointer-width WPARAM/LPARAM. On 32-bit, it is 12 bytes.

### NotifyIME

```c
BOOL NotifyIME(HIMC hIMC, DWORD dwAction, DWORD dwIndex, DWORD dwValue);
```

Sends notifications to the IME. Common actions:

| `dwAction` | Constant | Purpose |
|---|---|---|
| `0x10` | `NI_OPENCANDIDATE` | Open candidate window |
| `0x11` | `NI_CLOSECANDIDATE` | Close candidate window |
| `0x13` | `NI_SELECTCANDIDATESTR` | Select a candidate by index |
| `0x14` | `NI_CHANGECANDIDATELIST` | Change the candidate list |
| `0x15` | `NI_COMPOSITIONSTR` | Composition string operation |
| | | — `dwIndex = CPS_COMPLETE` → commit composition |
| | | — `dwIndex = CPS_CANCEL` → cancel composition |

### ImeConfigure

```c
BOOL ImeConfigure(HKL hKL, HWND hWnd, DWORD dwMode, LPVOID lpData);
```

Opens the IME's configuration UI. SogouPY launches `SGTool.exe --appid=config`.

### ImeEscape

```c
LRESULT ImeEscape(HIMC hIMC, UINT uEscape, LPVOID lpData);
```

Private escape interface. SogouPY only handles escape code `4102`, writing a static wide string to `lpData`.

---

## COMPOSITIONSTRING Structure

The `COMPOSITIONSTRING` structure is the central data exchange area between the IME and the IMM. It resides in shared memory accessible via the HIMC.

```c
typedef struct tagCOMPOSITIONSTRING {
    DWORD dwSize;               // Total structure size
    
    // Composition string (preedit)
    DWORD dwCompReadAttrLen;
    DWORD dwCompReadAttrOffset;
    DWORD dwCompReadClsLen;
    DWORD dwCompReadClsOffset;
    DWORD dwCompReadStrLen;     // Characters (not bytes)
    DWORD dwCompReadStrOffset;
    DWORD dwCompAttrLen;
    DWORD dwCompAttrOffset;
    DWORD dwCompClsLen;
    DWORD dwCompClsOffset;
    DWORD dwCompStrLen;         // Characters (not bytes)
    DWORD dwCompStrOffset;
    
    // Cursor
    DWORD dwCursorPos;
    DWORD dwDeltaStart;
    
    // Result string (committed text)
    DWORD dwResultReadClsLen;
    DWORD dwResultReadClsOffset;
    DWORD dwResultReadStrLen;   // Characters (not bytes)
    DWORD dwResultReadStrOffset;
    DWORD dwResultClsLen;
    DWORD dwResultClsOffset;
    DWORD dwResultStrLen;       // Characters (not bytes) — NOT bytes!
    DWORD dwResultStrOffset;
    
    // Private data
    DWORD dwPrivateSize;
    DWORD dwPrivateOffset;
} COMPOSITIONSTRING;
```

**Important:** `dwResultStrLen` and `dwCompStrLen` are measured in **characters** (WCHARs), not bytes. This is confirmed by NT5src (`ntuser/imm/ctxtinfo.c`). A common bug is to divide by 2, which reads only half the string.

### Querying the Composition String

```c
// Get preedit text
LONG len = ImmGetCompositionStringW(hIMC, GCS_COMPSTR, NULL, 0);  // Returns bytes
WCHAR* buf = malloc(len + sizeof(WCHAR));
ImmGetCompositionStringW(hIMC, GCS_COMPSTR, buf, len);

// Get committed text
LONG len = ImmGetCompositionStringW(hIMC, GCS_RESULTSTR, NULL, 0);
WCHAR* buf = malloc(len + sizeof(WCHAR));
ImmGetCompositionStringW(hIMC, GCS_RESULTSTR, buf, len);
```

**GCS_* flags:**

| Flag | Value | Description |
|---|---|---|
| `GCS_COMPSTR` | `0x0008` | Composition string (preedit) |
| `GCS_COMPATTR` | `0x0010` | Composition attributes |
| `GCS_COMPREADSTR` | `0x0001` | Composition reading string |
| `GCS_RESULTSTR` | `0x0800` | Result string (committed text) |
| `GCS_RESULTREADSTR` | `0x0200` | Result reading string |
| `GCS_CURSORPOS` | `0x0080` | Cursor position |
| `GCS_DELTASTART` | `0x0100` | Delta start position |

---

## Candidate List Structure

```c
typedef struct tagCANDIDATELIST {
    DWORD dwSize;               // Total structure size in bytes
    DWORD dwStyle;              // Candidate style (IME_CAND_*)
    DWORD dwCount;              // Number of candidates in current list
    DWORD dwSelection;          // Currently selected candidate index
    DWORD dwPageStart;          // First candidate on current page
    DWORD dwPageSize;           // Candidates per page
    DWORD dwOffset[1];          // Variable-length array of offsets from start of structure
    // Candidate strings follow after the offset array
} CANDIDATELIST;
```

### Querying Candidates

```c
// Get buffer size
DWORD size = ImmGetCandidateListW(hIMC, 0, NULL, 0);  // dwIndex=0 = first list
if (size > 0) {
    CANDIDATELIST* cl = malloc(size);
    ImmGetCandidateListW(hIMC, 0, cl, size);
    for (DWORD i = 0; i < cl->dwCount; i++) {
        WCHAR* candidate = (WCHAR*)((BYTE*)cl + cl->dwOffset[i]);
        // Use candidate text
    }
    free(cl);
}
```

**Gotcha:** `totalSize > 0` can coexist with `dwCount == 0`. Always check `dwCount` before iterating.

**Sogou quirk:** `dwOffset[0..N]` may all be zero. Actual candidate data is shifted 24 bytes further into the buffer. This is a non-standard layout.

---

## HIMC and HWND Lifecycle

### Input Method Context (HIMC)

An HIMC (Handle to Input Method Context) represents an active IME session. It contains:
- The COMPOSITIONSTRING shared memory
- Candidate lists
- Private IME data (size declared via `ImeInquire`)
- Status and conversion mode flags

```c
// Create a new context
HIMC himc = ImmCreateContext();

// Associate with a window
ImmAssociateContextEx(hwnd, himc, 0);

// Lock to access COMPOSITIONSTRING
LPINPUTCONTEXT lpIMC = ImmLockIMC(himc);
LPCOMPOSITIONSTRING lpCS = (LPCOMPOSITIONSTRING)ImmLockIMCC(lpIMC->hCompStr);
// ...read/write composition data...
ImmUnlockIMCC(lpIMC->hCompStr);
ImmUnlockIMC(himc);

// Cleanup
ImmDestroyContext(himc);
```

### Window (HWND) Requirements

The HWND associated with an HIMC must:
1. **Exist on the calling thread** — Win32 IMM functions are thread-affine
2. **Process messages** — `GetMessage`/`TranslateMessage`/`DispatchMessage` loop required
3. **Handle WM_IME_* messages** — at minimum via `DefWindowProc`

For headless operation (no visible UI), use a message-only window:

```c
HWND hwnd = CreateWindowExW(
    0,
    L"STATIC",           // Simple window class
    L"IME Host",
    0,                   // No visible style
    0, 0, 0, 0,
    HWND_MESSAGE,        // Message-only — no display needed
    NULL, NULL, NULL
);
```

---

## WM_IME_* Message Flow

When `ImeToAsciiEx` generates TRANSMSG entries, they are dispatched as window messages:

### Composition Messages

```
WM_IME_STARTCOMPOSITION          // Composition begins
WM_IME_COMPOSITION               // Composition updated
  lParam flags:
    GCS_COMPSTR      = 0x0008    // Preedit text changed
    GCS_RESULTSTR    = 0x0800    // Text committed
    GCS_CURSORPOS    = 0x0080    // Cursor moved
WM_IME_ENDCOMPOSITION            // Composition ends
```

### Candidate Messages

```
WM_IME_NOTIFY
  wParam:
    IMN_OPENCANDIDATE    = 0x05  // Candidate window opened
    IMN_CLOSECANDIDATE   = 0x06  // Candidate window closed
    IMN_CHANGECANDIDATE  = 0x07  // Candidate list updated
    IMN_SETCANDIDATEPOS  = 0x09  // Candidate window position changed
```

### Character Messages

```
WM_CHAR                          // Direct character output (bypassing composition)
WM_IME_CHAR                      // IME-generated character
```

### Processing Order

For a typical keystroke that produces composition:

```
1. ImeProcessKey() → TRUE (key accepted)
2. ImeToAsciiEx() → N messages
3. Messages posted:
   a. WM_IME_STARTCOMPOSITION (if new composition)
   b. WM_IME_COMPOSITION | GCS_COMPSTR (preedit update)
   c. WM_IME_NOTIFY | IMN_CHANGECANDIDATE (candidate list changed)
4. Application receives messages via GetMessage loop
```

For a keystroke that produces a commit:

```
1. ImeProcessKey() → TRUE
2. ImeToAsciiEx() → messages
3. Messages posted:
   a. WM_IME_COMPOSITION | GCS_RESULTSTR (result ready)
   b. WM_IME_ENDCOMPOSITION
   c. WM_IME_CHAR or WM_CHAR (committed character)
```

**Important:** `GCS_RESULTSTR` may persist in the COMPOSITIONSTRING after a commit. Do NOT re-read it unconditionally — only read when `WM_IME_COMPOSITION` with `GCS_RESULTSTR` flag is received, and consume it (clear your local copy) immediately.

---

## VK Mapping for Rime Integration

### The Mapping Problem

Rime uses XKB-style keysyms:
- Lowercase `a` = `0x61`
- Return = `0xff0d`
- BackSpace = `0xff08`

Windows IMM uses `VIRTUAL_KEY` codes:
- `VK_A` = `0x41`
- `VK_RETURN` = `0x0D`
- `VK_BACK` = `0x08`

### Mapping Strategy

For printable ASCII characters, use `VkKeyScanW`:

```c
SHORT result = VkKeyScanW((WCHAR)rime_keycode);
BYTE vk = LOBYTE(result);          // Virtual key code
BYTE shift_state = HIBYTE(result); // 0=normal, 1=shift, 2=ctrl, 4=alt
```

For special keys (Return, BackSpace, etc.), use a static lookup table.

### Building lParam (Key Data)

```c
UINT scan_code = MapVirtualKeyW(vk, MAPVK_VK_TO_VSC);
LPARAM lParam = (scan_code << 16)   // bits 16-23: scan code
              | 1;                    // bits 0-15: repeat count
// For key-up:
lParam |= (1 << 30) | (1 << 31);   // bit 30: previous state, bit 31: transition
```

### Building lpbKeyState

```c
BYTE keyState[256] = {0};
if (rime_modifier & SHIFT_MASK)
    keyState[VK_SHIFT] = 0x80;
if (rime_modifier & CONTROL_MASK)
    keyState[VK_CONTROL] = 0x80;
if (rime_modifier & ALT_MASK)
    keyState[VK_MENU] = 0x80;
if (is_shifted_char)
    keyState[VK_SHIFT] = 0x80;
```

---

## Thread Affinity Requirements

Win32 IMM functions are thread-affine. Key rules:

1. **HWND ownership:** A window can only be manipulated (sent messages, destroyed) by the thread that created it.

2. **Message pump:** `GetMessage`/`DispatchMessage` only delivers messages to windows owned by the calling thread.

3. **IME calls:** `ImeProcessKey`, `ImeToAsciiEx`, and `NotifyIME` must be called on the thread that owns the associated HWND.

4. **Context association:** `ImmAssociateContextEx` must be called on the thread that owns the HWND.

### Implications for Async Servers

In an async server (e.g., tokio), gRPC handler tasks run on arbitrary thread pool threads. You cannot call IMM functions directly from a handler.

**Solution:** Dedicate a single OS thread to all Win32 IMM operations. Use channels (e.g., `tokio::sync::oneshot`) to dispatch requests from async handlers to the dedicated thread and receive results.

```
[tokio handler task] --request--> [channel] --> [dedicated Win32 thread]
                     <--response-- [oneshot] <-- [processes IMM calls]
```

---

## Wine-Specific Workarounds

### Wine vs Real Windows

Wine implements the IMM32 API, but with differences:

| Feature | Real Windows | Wine |
|---|---|---|
| IME loading | Automatic via registry + HKL | Must call `LoadLibraryW` + exports manually |
| `ImeInquire` | Called by system | Must call explicitly |
| `ImeSelect` | Called on focus change | Must call explicitly |
| Message pump | System message loop | Must create custom loop |
| Desktop mode | Full IMM via csrss | IMM partially functional |

### Required Wine Environment Variables

```bash
export WINEPREFIX=~/.win32          # 32-bit prefix (or ~/.wine64 for 64-bit IMEs)
export WINEARCH=win32               # Force 32-bit (omit for 64-bit prefix)
export WINEDEBUG=-all               # Suppress debug output
export WINEDLLOVERRIDES="mscoree=d;mshtml=d"  # Suppress .NET/MSHTML dialogs
# DISPLAY=:99 only needed for IME installers; host process uses HWND_MESSAGE
```

### Wine IME Framework Limitations

1. Wine's `imm32.dll` does not automatically load IME DLLs. The host must `LoadLibraryW` the DLL and resolve exports via `GetProcAddress`.

2. `ImmSetActiveContext` and `WM_IME_SETCONTEXT` are not fully implemented. The host must call `ImeSelect` directly.

3. Some IMEs expect `TranslateMessage` to be called in the message loop. Without it, certain keyboard-related messages may not be generated.

4. Wine's `GetKeyboardLayout` returns a generic HKL that does not correspond to the loaded IME.

### PE Version Patching

Some IMEs (notably Sogou) detect Windows version via PE resource versions rather than API calls. Wine's system DLLs report Wine-era versions (10.0+), which can trigger undesirable code paths.

Fix: Patch the PE version resources of Wine's `kernel32.dll`:

```bash
# Change to Windows 7 SP1
rcedit ~/.win32/drive_c/windows/system32/kernel32.dll \
  --set-product-version "6.1.7601.17514" \
  --set-file-version "6.1.7601.17514"
```
