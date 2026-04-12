# Reverse Engineering Reference

> Standalone reference for Windows IME reverse engineering findings.
> Covers SogouPY, QQPinyin, and WINABC behaviors under Wine.

---

## SogouPY.ime Binary Profile (Sogou 10.5b, 32-bit)

| Property | Value |
|---|---|
| Installer | `sogou_pinyin_105b_xp.exe` (Inno Setup) |
| Architecture | 32-bit PE |
| Base address | `0x10000000` |
| Image size | `0x694000` |
| MD5 | `7022bd95...` |
| `sys/` vs `syswow64/` | Identical (single reverse track needed) |
| Data directory | `app/10.5.0.4737/` (version-specific) |

The DLL depends on: IMM32, COM/OLE, Named Pipes, WinHTTP/WinINet, and various Windows system services.

A 64-bit variant also exists (Sogou 16.3, PE32+ x64, ~52730 functions in IDA) with different internal offsets.

---

## IME Export Map

All Windows IME DLLs export a standard set of DDI (Device Driver Interface) functions:

| Export | Address (SogouPY) | Prototype | Internal Chain |
|---|---|---|---|
| `ImeInquire` | `0x10085260` | `BOOL(LPIMEINFO, LPTSTR, DWORD)` | → `sub_10085930` |
| `ImeSelect` | `0x10085500` | `BOOL(HIMC, BOOL)` | → `sub_10085FC0` |
| `ImeProcessKey` | `0x10085680` | `BOOL(HIMC, UINT, LPARAM, LPBYTE)` | → `sub_100862F0` → `sub_1009B770` |
| `ImeToAsciiEx` | `0x10085750` | `UINT(UINT, UINT, LPBYTE, LPTRANSMSGLIST, UINT, HIMC)` | → `sub_100865A0` → `sub_1009BC60` |
| `NotifyIME` | `0x10085820` | `BOOL(HIMC, DWORD, DWORD, DWORD)` | → `sub_100868E0` → `sub_1009C170` |
| `ImeConfigure` | `0x10085330` | `BOOL(HKL, HWND, DWORD, LPVOID)` | → `sub_10085BE0` (launches `SGTool.exe`) |
| `ImeEscape` | `0x100853F0` | `LRESULT(HIMC, UINT, LPVOID)` | Only handles cmd `4102` |

**Required call sequence for host integration:**

```
ImeInquire(...)          # Query capabilities
ImeSelect(himc, TRUE)    # Activate IME on context
loop:
  ImeProcessKey(himc, vk, lParam, keyState)  # Test if key is handled
  if accepted:
    ImeToAsciiEx(vk, scan, keyState, transMsgList, 0, himc)  # Process key
    // Parse TRANSMSG results
    NotifyIME(himc, ...)  # Send notifications (candidate selection, etc.)
ImeSelect(himc, FALSE)   # Deactivate
```

---

## Internal Function Semantics

### Global Guard

`byte_1059C7D4` — when nonzero, all seven exports return 0 immediately. Acts as a global kill switch.

### Central Dispatcher

`sub_10100AC0` — large switch statement over notify/event codes. Controls the composition lifecycle: start, update, cancel, commit. References the internal string `ImeContext::NotifyIME_CompositionStr`.

### Mode/State Selector

`sub_1009D3F0` — called by all three deep handlers (`sub_1009B770`, `sub_1009BC60`, `sub_1009C170`). Returns a table/index-derived mode value or fallback `9`. Purpose: selects processing behavior based on IME state.

### IMC I/O Gates

- `sub_10097F20` (read gate) — reads from input method context via `sub_100F04B0(..., mode=15)`, returns 1 on success
- `sub_10098020` (write gate) — writes to context via `sub_100F1400(...)`, resets guard `dword_1059968C = 0` on failure

---

## The +0x36c Activation Gate

**The most important reverse engineering finding for Wine integration.**

`sub_100F6080` reads `t_dataPrivate + 0x36c` during `ImeProcessKey`. Under the C host (and Wine generally), this byte is always 0, causing `sub_100801D0(code=0x205)` to return 0 — which makes `ImeProcessKey` reject all keys.

**Force-activation PoC:**

```gdb
# force36c_probe.gdb
break *0x100F6080
commands
  set *(char*)($eax + 0x36c) = 1
  continue
end
```

With the byte patched to 1, `ImeProcessKey` returns 2 (non-standard truthy value) and `TRACE nihao` produces real preedit text.

**Natural activation:** `sub_100F6110` is the natural entry that writes `1` to `+0x36c`. It is called during Sogou's initialization when installed properly (via official installer). When the IME DLL is deployed manually without running the installer, this initialization never occurs.

**Solution in practice:** Install Sogou via its official installer into the Wine prefix. The activation gate is set during IME installation/registration.

---

## Sogou Version Detection and Code Path Split

*Applies to both 32-bit (Sogou 10.5b) and 64-bit (Sogou 16.3) variants. All IDA addresses below are from the 64-bit binary.*

### The Problem

Sogou uses two completely different code paths for outputting text:

| Path | Condition | Mechanism | Works with Host? |
|---|---|---|---|
| Win7 (standard) | `byte_3554 = 0` | Standard IMM: writes to COMPOSITIONSTRING, fills `hMsgBuf`, calls `ImmGenerateMessage()` | Yes |
| Win8+ (custom) | `byte_3554 = 1` | Custom: `SendMessageW(hWnd, 0x8BB8, count, transmsg_ptr)` — bypasses COMPOSITIONSTRING entirely | No |

### Version Detection Logic

`sub_1801BECD0` (64-bit) detects Windows version:

```
version = max(
    get_pe_file_version("kernel32.dll"),   // FileVersion from .rsrc
    RtlGetNtVersionNumbers()               // NT version from ntdll
)
if version.major >= 6 && version.minor >= 2:    // Win8+
    byte_3554 = 1
```

Wine's `kernel32.dll` reports PE version `10.0.xxxxx`, which always triggers the Win8+ path. The `winecfg` version setting does NOT affect the PE version embedded in DLL resources.

### byte_3554 Comprehensive Analysis (64-bit)

`byte_3554` is at offset `0xDE2` in the 64-bit binary. Beyond the Win7/Win8+ dispatch split, it affects multiple code paths:

| Code Path | byte_3554=0 (Win<8) | byte_3554=1 (Win≥8) |
|---|---|---|
| SetDataToIMC guard in ImeToAsciiEx | Writes to COMPOSITIONSTRING before processing | **SKIPPED** |
| SetDataToIMC guard in ImeProcessKey | Writes with additional byte_3582 guard | Skipped |
| PostMessage fallback in ImeToAsciiEx | Active (byte_3640 + sub_1801BE510) | Skipped |
| NotifyIME composition state reset | Normal toggle logic | Skipped (byte_3581=1 override) |
| TRANSMSG dispatch | `hMsgBuf + ImmGenerateMessage(hIMC)` | `SendMessageW(hWnd, 0x8BB8, count, data)` |

**Related offset map** (64-bit binary):

| Offset | Decimal | Purpose |
|---|---|---|
| 0x8 | 8 | Legacy pre-Vista flag |
| 0x9 | 9 | Vista+ flag (major ≥ 6) |
| 0xA | 10 | iexplore.exe detection |
| 0xB | 11 | Win10+ flag |
| 0xDE2 | 3554 | **Win8+ flag** — controls IMC sync + message dispatch |
| 0xDF6 | 3574 | TRANSMSG template size control |
| 0xDFB | 3579 | TRANSMSG processing context flag |
| 0xDFD | 3581 | NotifyIME composition guard |
| 0xDFE | 3582 | ImeProcessKey SetDataToIMC guard |
| 0xE28 | 3624 | PostMessage(0x83FA) trigger |
| 0xE31 | 3633 | IME_SETOPEN SendMessage trigger |
| 0xE38 | 3640 | PostMessage fallback condition |

**Critical finding:** `byte_3554` controls not just TRANSMSG dispatch but also COMPOSITIONSTRING writing. The `CL_SetDataToIMCC` / `CL_SetDataToContext` pre-synchronization is skipped in Win8+ mode.

### COMPOSITIONSTRING Writing: sub_18014CB70

This function writes engine data → INPUTCONTEXT after key processing, regardless of `byte_3554`:

1. `ImmLockIMC`
2. Scan command list for type-0 commands with case 3/6/8
3. IF found AND `hPrivate.vtable[7](count)` AND `hPrivate.vtable[5]()`:
   → Call `hCompStr.vtable[48]` (offset 0x180) = **populate result string**
4. UNCONDITIONALLY: `SetDataToIMCC` for `hCompStr`, `hCandInfo`, `hPrivate`
5. `SetDataToContext` for INPUTCONTEXT fields
6. `ImmUnlockIMC`

**Why case 3 works but case 8 doesn't:**
- Case 3 (`nihao+space`): active composition → `hPrivate` conditions pass → `vtable[48]` populates result
- Case 8 (standalone punctuation): no prior composition → `hPrivate` conditions fail → `vtable[48]` skipped → result string empty

### IDA Call Chain (64-bit)

```
ImeToAsciiEx
  → sub_18012B5F0
    → sub_180154360
      → sub_18014D1A0
        → sub_1802F93F0
          → sub_1802F8A60    ← Two-branch dispatch point
            if byte_3554 == 0: standard IMM path
            if byte_3554 == 1: SendMessageW(hWnd, 0x8BB8, ...)
```

### Fix: PE Version Patching

Patch Wine's `kernel32.dll` PE version to Windows 7 SP1:

```bash
# Using rcedit:
rcedit ~/.win32/drive_c/windows/system32/kernel32.dll \
  --set-product-version "6.1.7601.17514" \
  --set-file-version "6.1.7601.17514"

# Or using custom Python script:
python patch_pe_version.py ~/.win32/drive_c/windows/system32/kernel32.dll 6.1.7601
```

This forces `byte_3554 = 0`, activating the standard IMM path that our host can intercept.

---

## IME Behavioral Quirks

### SogouPY

| Quirk | Detail |
|---|---|
| `+0x36c` activation gate | Must be set to 1 for `ImeProcessKey` to accept keys |
| `ImeProcessKey` returns 2 | Non-standard truthy value (not `TRUE` = 1) |
| `byte_3554` version gate | Controls standard vs custom output path |
| `0x8BB8` private message | Custom window message for Win8+ punctuation dispatch |
| Registry path doubling | Setting root to `Z:\opt\sogou\app\10.5.0.4737` causes Sogou to look for `.../10.5.0.4737/10.5.0.4737/` |
| CANDIDATELIST offset shift | `dwOffset[0..N]` are all zeros; actual candidate data is shifted 24 bytes further into the buffer |
| Data path dependency | Requires `app/10.5.0.4737/` directory with dictionaries and resources |

### QQPinyin

| Quirk | Detail |
|---|---|
| Standard IMM compliance | Follows IMM DDI conventions closely |
| V-mode | Expression input (date, math) via `v` prefix |
| U-mode | Unicode/radical input via `u` prefix |
| UserCenter.exe | Spawns companion IPC process; IME works even if it fails |
| Stable under Wine | More reliable than Sogou for key processing and candidate retrieval |

### WINABC

| Quirk | Detail |
|---|---|
| Pseudo-Unicode candidates | Returns GBK-encoded bytes in WCHAR positions (e.g., U+E3C4 = GBK 0xC4E3 = "你") |
| Codepage decoding required | Split WCHAR into byte pairs, decode via CP936/GBK |
| Basic but reliable | Always activates, always processes keys — good for baseline testing |

---

## TRANSMSG Structure Details

### 32-bit Layout

```c
typedef struct tagTRANSMSG {
    UINT    message;    // 4 bytes — WM_IME_COMPOSITION, WM_CHAR, etc.
    WPARAM  wParam;     // 4 bytes
    LPARAM  lParam;     // 4 bytes
} TRANSMSG;             // Total: 12 bytes
```

### 64-bit Layout

```c
typedef struct tagTRANSMSG {
    UINT    message;    // 4 bytes
    // 4 bytes padding (alignment to 8-byte)
    WPARAM  wParam;     // 8 bytes (pointer-sized)
    LPARAM  lParam;     // 8 bytes (pointer-sized)
} TRANSMSG;             // Total: 24 bytes
```

**Bug encountered:** 64-bit Sogou writes 24-byte TRANSMSG entries. Code initially parsed 12-byte entries, reading garbage for the second half of each message. Fixed by using official `windows::Win32::UI::Input::Ime::{TRANSMSG, TRANSMSGLIST}` types.

---

## NT5src IMM Pipeline Reference

The authoritative IMM processing pipeline from `nt5src/Source/XPSP1/NT/windows/core/ntuser/imm/input.c`:

### Nine-Stage Model

| Stage | Marker | NT5src Anchor | Description |
|---|---|---|---|
| S0 | `RESET_SESSION` | Context activation | Session reset entry point |
| S1 | `ACTIVATE_CONTEXT` | `ImmSetActiveContext` | Sends `WM_IME_SETCONTEXT`, calls `NtUserNotifyIMEStatus` |
| S2 | `KEY_ENTRY` | Pre-`ImmProcessKey` | Key event ingress |
| S3 | `IMM_PROCESS_TRANSLATE` | `ImmProcessKey` → `ImmTranslateMessage` | Core processing handoff |
| S4 | `IME_EXPORT_CALLS` | `ImeToAsciiEx` output | TRANSMSG generation |
| S5 | `DISPATCH_COMPLETE` | Message dispatch | Per-keystroke message delivery |
| S6 | `IMM_SNAPSHOT` | Candidate list retrieval | State snapshot for query |
| S7 | `EVENT_RESULT` | Per-key summary | Backend state after processing |
| S8 | `QUERY_RESULT` | Query boundary | Final candidate query result |

### Key NT5src Findings

1. **`dwResultStrLen` is in characters (WCHARs), not bytes.** Source: `ctxtinfo.c`. This caused a bug where code divided by 2, reading only half the committed string.

2. **`totalSize > 0` can coexist with `dwCount == 0`** in candidate lists. Don't assume non-zero size means non-zero candidates.

3. **TRANSMSG overflow handling:** If `ImeToAsciiEx` returns more messages than the inline buffer can hold, NT5src consumes `hMsgBuf` from the IMC (input method context).

4. **Console IME model (`conime.c`):** Hidden window + message loop processing `WM_IME_*` messages. This was the architectural reference for the host design.

---

## Probe Tools

### ime_probe.c

**Purpose:** Static export validation — confirms DLL exports match reverse engineering findings.

**What it does:**
1. `LoadLibraryA("SogouPY.ime")`
2. `GetProcAddress` for all 6 key exports
3. Prints addresses (for cross-reference with IDA)
4. Calls `ImeInquire` — dumps `fdwProperty`, `fdwConversionCaps`, `fdwSentenceCaps`, UI class name
5. Calls `ImeEscape(4102)` — dumps output text

**Build:** `i686-w64-mingw32-gcc -o ime_probe.exe ime_probe.c -limm32`

### ime_flow_probe.c

**Purpose:** Minimal lifecycle flow test — attempts the full export call sequence without a Win32 host.

**What it does:**
1. Creates `ImmCreateContext()`
2. Calls `ImeSelect(himc, TRUE)`
3. Calls `ImeProcessKey(himc, VK_A, ...)` with populated keyboard state
4. Calls `ImeToAsciiEx(VK_A, ...)` with 16-slot TRANSMSGLIST
5. Calls `NotifyIME(NI_COMPOSITIONSTR, CPS_COMPLETE)`
6. Teardown

**Result:** All returns are 0 — confirms that a full Win32 host (with HWND and message pump) is required. The probe cannot substitute for a real window.

**Build:** `i686-w64-mingw32-gcc -o ime_flow_probe.exe ime_flow_probe.c -limm32`

---

## VK Mapping Knowledge

The bridge must convert between Rime/XKB keysyms and Win32 virtual key infrastructure.

### Required Win32 Key Structures

For `ImeProcessKey(himc, vKey, lParam, lpbKeyState)`:

| Parameter | How to Build |
|---|---|
| `vKey` | Map from ASCII char via `VkKeyScanW(char)`, extract low byte for VK code, high byte for shift state |
| `lpbKeyState` | 256-byte array. Set `[VK_SHIFT]`, `[VK_CONTROL]`, `[VK_MENU]` based on Rime modifier mask |
| `lParam` (lKeyData) | Bits 16–23 = scan code (from `MapVirtualKeyW(vk, MAPVK_VK_TO_VSC)`), bit 30 = previous state, bit 31 = transition state |

**Critical:** If `lParam == 0` (no scan code), `ImeProcessKey` returns 0 for most IMEs.

### Rime Keysym → VK Mapping Table (abbreviated)

| Rime Keysym | Win32 VK | Notes |
|---|---|---|
| `0x61–0x7a` (a–z) | `VK_A..VK_Z` (0x41–0x5A) | Not shifted |
| `0x41–0x5a` (A–Z) | `VK_A..VK_Z` | Shifted |
| `0x30–0x39` (0–9) | `VK_0..VK_9` | |
| `0xff0d` (Return) | `VK_RETURN` (0x0D) | |
| `0xff08` (BackSpace) | `VK_BACK` (0x08) | |
| `0xff1b` (Escape) | `VK_ESCAPE` (0x1B) | |
| `0x20` (Space) | `VK_SPACE` (0x20) | |
| `0xff09` (Tab) | `VK_TAB` (0x09) | |
| `0x2c` (comma) | `VK_OEM_COMMA` (0xBC) | |
| `0x2e` (period) | `VK_OEM_PERIOD` (0xBE) | |

---

## Wine-Specific Issues

| Issue | Detail | Workaround |
|---|---|---|
| Desktop mode insufficient | Wine desktop cannot provide full IMM lifecycle | Build dedicated Win32 host with HWND + message pump |
| PE version mismatch | kernel32.dll reports 10.0, triggering Win8+ code paths | Patch PE version to 6.1.7601 |
| `winecfg` version ineffective | Does not change PE version in DLL resources | Must use `rcedit` or binary patching |
| Wine config dialog | Appears on first run; blocks automation | `WINEDLLOVERRIDES="mscoree=d;mshtml=d"` |
| Display required | Some IME operations need a display | Xvfb for installers only; host process uses `HWND_MESSAGE` |
| 32-bit prefix mandatory | IME DLLs are 32-bit | `WINEARCH=win32` |
| `WINEDEBUG` noise | Massive diagnostic output | `WINEDEBUG=-all` for production |
| `VkKeyScanExW` availability | Wine implements this correctly | Discovered during Phase 3 as the key mapping solution |
