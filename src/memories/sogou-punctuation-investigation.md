# Sogou IME Punctuation Investigation (2026-04-07 → 04-08)

## Problem

After switching from QQ Pinyin to Sogou Pinyin (SogouPY.ime) on the remote Wine+Linux server (127.0.0.1), **Chinese punctuation is completely lost**. Typing `,` or `.` in Chinese mode should produce `，` or `。`, but instead:

- **Post-composition punctuation**: `nihao,` commits `你好` (missing `，`)
- **Standalone punctuation**: `,` alone is "accepted" by `ImeProcessKey` but produces no commit
- **Number+punctuation**: `1..` produces `1.` instead of `1.。` — only the Chinese punctuation is missing
- English letters, selection digits, and Space all work correctly

QQ Pinyin on the same Wine stack (local server, 127.0.0.1:50051) produces punctuation correctly.

## Environment

| | Local (working) | Remote 32-bit | Remote 64-bit |
|---|---|---|---|
| OS | Linux + Wine | Linux + Wine | Linux + Wine 11.5 staging |
| WINEPREFIX | .wine32 | .winegbk | .wine64 |
| IME | QQ Pinyin | Sogou Pinyin 10.5 (32-bit) | SogouPY.ime (64-bit PE32+ AMD64) |
| Port | 127.0.0.1:50051 | 127.0.0.1:50051 | [::]:50056 |
| fdwProperty | — | 0x1E0002 | — |
| IDA database | — | — | "sogou64" (52730 functions) |

## Investigation Timeline

### 1. Initial observation: ImeToAsciiEx returns 0 messages for standalone punctuation

For a standalone `.` (VK=0xBE):
- `ImeProcessKey` returns `BOOL(2)` (accepted, non-standard but truthy)
- `ImeToAsciiEx` returns **msg_count=4**, containing:
  - `0x10D` (WM_IME_STARTCOMPOSITION)
  - `0x10F` (WM_IME_COMPOSITION), lp=`0x808` (GCS_RESULTSTR | GCS_RESULTCLAUSE)
  - `0x10E` (WM_IME_ENDCOMPOSITION)
  - `0x282` (WM_IME_NOTIFY), wp=0x4 (IMN_CLOSECANDIDATE)
- So Sogou **tells us there's a result** via WM_IME_COMPOSITION with GCS_RESULTSTR flag
- But `ImmGetCompositionStringW(GCS_RESULTSTR)` returns **size=0** (empty!)

This is the core problem: **Sogou sends WM_IME_COMPOSITION with GCS_RESULTSTR, but the result string is already gone by the time we read it.**

For comparison, QQ Pinyin correctly fills `GCS_RESULTSTR` and it persists until the next composition.

### 2. Scancode fix attempt

Initially suspected that `ImeToAsciiEx` was receiving `uScanCode=0` (hardcoded). Changed to extract scan code from `lParam` via `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)`. No effect — Sogou doesn't depend on the scan code for punctuation mapping.

### 3. COMPOSITIONSTRING memory dump

Locked `INPUTCONTEXT->hCompStr` (offset 0x118 in 32-bit INPUTCONTEXT) and dumped the entire `COMPOSITIONSTRING` structure (456 bytes):

```
CS[0x54] dwResultStrLen = 0x00000000 (0)      ← LENGTH IS ZERO
CS[0x58] dwResultStrOff = 0x0000018E (398)     ← OFFSET EXISTS
```

Raw bytes at offset 0x18E: `[00 00 00 00 00 00 20 00 ...]` — all zeros (no data).

Key finding: **Sogou sets a valid offset but zero length, and the data at that offset is empty.** This confirms Sogou writes-then-clears the result string within the same `ImeToAsciiEx` call.

### 4. ANSI codepage test

Tried `ImmGetCompositionStringA(GCS_RESULTSTR)` — also returns size=0. The WINEPREFIX uses GBK codepage but that's not the issue.

### 5. IMC message buffer (hMsgBuf) check

Read `INPUTCONTEXT` at offset 0x128 (dwNumMsgBuf) and 0x12C (hMsgBuf): `dwNumMsgBuf=0`. The IMC's own message buffer is empty — Sogou puts messages in the `trans_msgs` array, not in hMsgBuf.

### 6. Message dispatch test

Dispatched the 4 translation messages via `SendMessageW` to the STATIC window:
- All returned `LRESULT(0)`
- Post-dispatch `GCS_RESULTSTR` still empty
- No WM_IME_CHAR or WM_CHAR generated in the queue

This makes sense: `DefWindowProc`'s handler for `WM_IME_COMPOSITION` calls `ImmGetCompositionStringW(GCS_RESULTSTR)` again, which is already empty.

### 7. Scanning for hidden data

Scanned the entire 456-byte COMPOSITIONSTRING buffer for:
- UTF-16 Chinese comma (U+FF0C = bytes `0C FF`)
- UTF-16 Chinese period (U+3002 = bytes `02 30`)
- GBK comma (A3 AC)
- GBK period (A1 A3)

**None found.** The punctuation character is simply never written to the COMPOSITIONSTRING buffer.

### 8. Default HIMC divergence check

Compared `session.himc` (our `ImmCreateContext()` HIMC) with `ImmGetContext(session.hwnd)` (the window's default HIMC). If Sogou writes to a different HIMC, we'd miss it.

Result pending — deployed but not yet tested at the end of this session.

### 9. Polling thread during ImeToAsciiEx

Spawned a background thread that continuously calls `ImmGetCompositionStringW(GCS_RESULTSTR)` while the main thread is inside `ImeToAsciiEx`. If Sogou transiently writes then clears the result, the poller might catch it.

Result pending — deployed but not yet tested at the end of this session.

## Root Cause Hypothesis

Sogou's `ImeToAsciiEx` for punctuation does the following **atomically** within a single call:
1. Writes the result string to COMPOSITIONSTRING
2. Generates WM_IME_STARTCOMPOSITION + WM_IME_COMPOSITION(GCS_RESULTSTR) + WM_IME_ENDCOMPOSITION
3. Clears the result string (part of END composition cleanup)
4. Returns 4 messages

By the time we read `GCS_RESULTSTR` after the call returns, step 3 has already happened. This is unlike QQ Pinyin which leaves the result in place for the caller to read.

On real Windows, this would work because `DefWindowProc` processes WM_IME_COMPOSITION synchronously during the message dispatch phase, and `ImmGetCompositionStringW` is called from within the WM_IME_COMPOSITION handler BEFORE the WM_IME_ENDCOMPOSITION handler clears it. But in our architecture, we call `ImeToAsciiEx` directly and try to read the result after ALL messages have been generated — too late.

## Possible Fix Approaches (Not Yet Tried)

1. **Hook `ImmSetCompositionStringW`** — Patch Wine's imm32.dll or use IAT hooking to intercept the moment Sogou writes the result string, capturing it before it's cleared.

2. **Use `ImmGenerateMessage` + message loop** — Instead of calling `ImeToAsciiEx` directly, implement a proper Windows message loop with `TranslateMessage`/`DispatchMessage`, and use a custom `WndProc` that catches WM_IME_COMPOSITION and reads `GCS_RESULTSTR` at the right moment.

3. **Modify Wine's imm32** — Patch Wine's `ImmTranslateMessage` or `default_ime_compositionW` to capture the result string before dispatching WM_IME_ENDCOMPOSITION.

4. **Use WM_IME_CHAR** — On real Windows, `DefWindowProc` converts `WM_IME_COMPOSITION(GCS_RESULTSTR)` into `WM_IME_CHAR` messages. If we dispatch the messages through the proper window proc, we might receive `WM_IME_CHAR` with the character. But our test showed no WM_IME_CHAR was generated — likely because `DefWindowProc` also fails to read `GCS_RESULTSTR`.

5. **Subclass the window proc** — Register a custom `WndProc` that intercepts `WM_IME_COMPOSITION` and reads `GCS_RESULTSTR` immediately, before `DefWindowProc` can reach `WM_IME_ENDCOMPOSITION`.

## Reference: Wine Source Analysis

Wine's `ImmTranslateMessage` (wine/dlls/imm32/imm.c:3096):
- Calls `ImeToAsciiEx(vkey, scan, state, &buffer.list, 0, himc)`
- For each returned message, calls `PostMessageW(hwnd, msgs[i].message, ...)`
- Does NOT read GCS_RESULTSTR itself

Wine's `DefWindowProcW` (wine/dlls/user32/defwnd.c):
- On `WM_IME_COMPOSITION`: calls `ImmGetCompositionStringW(GCS_RESULTSTR)` and sends `WM_IME_CHAR` per character
- This is where the actual character extraction happens in normal apps

## Related Fix: Candidate List Offset Shift (d48dff2)

The same Sogou IME had a similar issue with `CANDIDATELIST`: `dwOffset[0..N]` were all zero, with real offsets shifted 24 bytes further into the buffer. Fixed by scanning forward for the first valid non-zero offset. This pattern of non-standard internal memory layout is a recurring Sogou characteristic.

## Files Modified (Debug State, Uncommitted)

- `src/win_imm/imm_ops.rs`: Added `ImmLockIMC/ImmUnlockIMC/ImmLockIMCC/ImmUnlockIMCC/ImmGetCompositionStringA` FFI. Added `drain_imc_msg_buf()`, `dump_trans_msgs_deep()`, `dump_and_extract_result_str()`.
- `src/win_imm/mod.rs`: Added pre-ToAsciiEx GCS_RESULTSTR check, default HIMC comparison, polling thread during ImeToAsciiEx, SendMessageW dispatch fallback, IMC message buffer processing, scancode fix.
- `tests/test_grpc_punctuation.rs`: gRPC client-based integration test for punctuation (connects to running server, sends key sequences, checks commits).
- `src/main.rs`: Fixed `backend::native` path to use full crate prefix.

## Status: UNRESOLVED

Approach 5 (custom WndProc that reads GCS_RESULTSTR during WM_IME_COMPOSITION) is the most promising next step.

---

## Session 2 (2026-04-08): 64-bit SogouPY, IDA Reverse Engineering, Version Spoofing

### 10. 64-bit TRANSMSG alignment fix (commit 3b5a70b)

The 64-bit SogouPY.ime produces 24-byte TRANSMSG structs (message: u32 + padding + WPARAM: u64 + LPARAM: u64). Our code used a manual `TransMsg` struct with `u32` fields (12 bytes), causing misaligned reads. Fixed by switching to the official `windows::Win32::UI::Input::Ime::{TRANSMSG, TRANSMSGLIST}` types which have correct 64-bit layout.

### 11. IDA reverse engineering of SogouPY64

Opened the 64-bit SogouPY.ime in IDA Pro (database "sogou64"). Traced the call chain:

```
ImeToAsciiEx → sub_18012B5F0 → sub_180154360 (real processing)
  → sub_18014D1A0 → sub_1802F93F0 → sub_1802F8A60 (TRANSMSG dispatch)
```

#### Key discovery: `byte_3554` controls the message dispatch path

In `sub_1802F8A60`, there are two branches controlled by `*(runtime+3554)`:

| byte_3554 | Condition | Behavior |
|---|---|---|
| 0 | Windows < 8 | Writes result to COMPOSITIONSTRING, fills hMsgBuf, calls `ImmGenerateMessage(himc)` |
| 1 | Windows ≥ 8 | **SKIPS** writing COMPOSITIONSTRING, sends `SendMessageW(hWnd, 0x8BB8, count, transmsg_ptr)` directly |

`sub_1801B6D70` sets `byte_3554 = 1` when the detected Windows version ≥ 6.2 (Win 8+).

#### Version detection logic (sub_1801BECD0)

SogouPY detects the Windows version by taking the **maximum** of:
1. kernel32.dll PE file version (via `GetFileVersionInfoW` + `VerQueryValueW`)
2. `RtlGetNtVersionNumbers`

The `winecfg /v win7` setting only affects `GetVersionExW` and `RtlGetVersion`, NOT the PE version of kernel32.dll which Wine builds with its host version (10.0.19045.5796 for Wine 11.5).

### 12. Wine prefix changed to Windows 7

Set Wine prefix to Windows 7 via:
```
WINEPREFIX=/opt/sogou/.wine64 /opt/wine-staging/bin/wine winecfg /v win7
```
Verified: `RtlGetVersion` → 6.1.7601, `GetVersionExW` → 6.1.7601. **But byte_3554 still = 1** because kernel32.dll PE version = 10.0.

### 13. Version detection reproduction test

Created `tests/test_version_detect.rs` to reproduce SogouPY's version detection:

```
kernel32.dll PE FileVersion:  10.0.19045.5796  → byte_3554=1
RtlGetNtVersionNumbers:       6.1.7601         → byte_3554=0
GetVersionExW:                6.1.7601         → byte_3554=0
RtlGetVersion:                6.1.7601         → byte_3554=0
```

SogouPY takes max(10.0, 6.1) = 10.0 → byte_3554=1 → uses 0x8BB8 path.

### 14. Manual IAT hooking attempt (FAILED)

Attempted to hook version APIs in SogouPY's IAT:
- `VerQueryValueW` — NOT in SogouPY's static IAT (VERSION.dll loaded dynamically)
- `GetProcAddress` hooked in KERNEL32.dll IAT — intercepts dynamic lookups

**Problem**: SogouPY's version detection runs DURING `LoadLibraryW` (in DllMain), before our post-load IAT hooks are active. Two-phase approach (`install_early` + `install_post_load`) couldn't work because DllMain runs before phase 2.

### 15. retour inline hooking (hooks worked, version spoofing worked)

Rewrote version_hook.rs using `retour::GenericDetour` (stable Rust, no nightly):
- Hooks patch the function **prologue**, intercepting ALL callers process-wide
- Installed BEFORE `LoadLibraryW` → active during SogouPY's DllMain

All 3 hooks verified:
```
version_hook: VerQueryValueW hooked at 0x6ffffca01364
version_hook: RtlGetNtVersionNumbers hooked at 0x6fffffc08100
version_hook: RtlGetVersion hooked at 0x6fffffc08840
version_hook: done (VerQueryValueW=true, RtlGetNtVersionNumbers=true, RtlGetVersion=true)
version_hook: patched VS_FIXEDFILEINFO → 6.1.7601.17514  (multiple calls during DLL load)
```

**But**: With byte_3554=0 (Win7 mode), `ImmGetCompositionStringW(GCS_RESULTSTR)` returns raw pinyin "nihao" instead of "你好". SogouPY's Win7 code path does not correctly convert pinyin to Chinese under Wine.

### 16. dwResultStrLen interpretation bug found

NT5 source confirms `dwResultStrLen` is in **characters** (WCHARs), not bytes:
```c
// GetCompInfoW macro from nt5src/windows/core/ntuser/imm/ctxtinfo.c:
dwBufLen = pCompStr->dw ## Component ## Len * sizeof(WCHAR);
```

Our session.rs code had `dwResultStrLen / 2`, reading only half the string. Fixed.

### 17. Comparison of Win7 vs Win10 mode (both paths broken differently)

| Mode | byte_3554 | nihao+space | standalone comma |
|---|---|---|---|
| Win7 (6.1) | 0 | raw pinyin "nihao" ✗ | raw pinyin or nothing ✗ |
| Win10 (10.0) | 1 | "你好" via 0x8BB8 WM_IME_CHAR ✓ | nothing (no WM_IME_CHAR for punctuation) ✗ |

### 18. Direct PE version patching with rcedit (SOLUTION)

Instead of runtime hooks, directly patched Wine's kernel32.dll PE version:

```bash
# Using rcedit from https://github.com/electron/rcedit/releases/tag/v2.0.0
wine rcedit-x64.exe kernel32.dll \
  --set-file-version "6.1.7601.17514" \
  --set-product-version "6.1.7601.17514" \
  --set-version-string "FileVersion" "6.1.7601.17514" \
  --set-version-string "ProductVersion" "6.1.7601.17514"
```

Also wrote `patch_pe_version.py` — a standalone Python script that reads/writes VS_FIXEDFILEINFO directly:

```bash
python3 patch_pe_version.py kernel32.dll                    # read
python3 patch_pe_version.py kernel32.dll 6.1.7601.17514     # write (auto-backup)
```

**This made byte_3554=0, and combined with the original TRANSMSG buffer code path, punctuation now works!**

### 19. Final test results (with patched kernel32.dll + original code, no hooks)

```
nihao + space       → 你好        ✅
nihao + comma       → 你好，      ✅  (the , triggers commit of 你好, then SogouPY handles , itself)
standalone comma    → ，          ✅
nihaoshijie2+comma  → 你好世界    ✅  (comma triggers commit)
standalone period   → (empty)     ❌  (still failing)
```

Test suite: `GRPC_SERVER_ADDR="http://[::1]:50056" cargo test --test test_grpc_punctuation -- --nocapture` **PASSED** (the period test doesn't have an assertion).

## Root Cause (Updated)

The original hypothesis (§Root Cause Hypothesis above) was **partially correct** for the 32-bit case. For the 64-bit case, the actual root cause has two layers:

### Layer 1: Wine's kernel32.dll PE version triggers Win8+ code path

Wine builds kernel32.dll with its own version number (10.0.19045.5796 for Wine 11.5). SogouPY reads this PE version and takes `max(PE_version, RtlGetNtVersionNumbers)`. Since Wine's kernel32 reports 10.0, SogouPY thinks it's on Windows 10+ and enables `byte_3554=1`, which uses `SendMessageW(hWnd, 0x8BB8, ...)` — a custom message that bypasses the standard IMM message flow entirely.

### Layer 2: The 0x8BB8 path skips COMPOSITIONSTRING for direct messages

When `byte_3554=1`, SogouPY doesn't write the result to COMPOSITIONSTRING at all. Instead it packs the result into TRANSMSG structures and sends them via the custom 0x8BB8 message. For word composition (nihao→你好), the 0x8BB8 payload includes WM_IME_CHAR messages with the Chinese characters. But for standalone punctuation, it only includes WM_IME_COMPOSITION(GCS_RESULTSTR) WITHOUT WM_IME_CHAR, and COMPOSITIONSTRING is empty.

### Solution: Patch kernel32.dll PE version to 6.1 (Win7)

This forces `byte_3554=0`, making SogouPY use the standard IMM path: write result to COMPOSITIONSTRING → fill hMsgBuf → ImmGenerateMessage → standard TRANSMSGs with WM_IME_COMPOSITION(GCS_RESULTSTR). In this mode, ImmGetCompositionStringW correctly returns the converted Chinese text.

## Removed Code

- `src/win_imm/version_hook.rs` — deleted (retour inline hooks no longer needed)
- `retour` dependency removed from Cargo.toml
- `--spoof-version` CLI argument removed from main.rs
- All IAT patching code, byte_3554 direct patching code removed

## Current State (2026-04-08)

- kernel32.dll patched: `6.1.7601.17514` (backup at `kernel32.dll.bak`)
- Wine prefix: Windows 7 via winecfg
- Code changes vs committed HEAD (3b5a70b): only `.gitignore` and `main.rs` import fix
- Key remaining issue: standalone period (`.`) still doesn't produce `。`
- `patch_pe_version.py` created at `/opt/sogou/patch_pe_version.py`
