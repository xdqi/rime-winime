# Phase 0 IME Export Contract (Draft)

Date: 2026-04-02
Binary: `SogouPY.ime` (base `0x10000000`)

This draft captures callable contract facts recovered from decompilation.

## 1) ImeProcessKey

- addr: `0x10085680`
- proto: `BOOL __stdcall(HIMC, UINT, LPARAM, const LPBYTE lpbKeyState)`
- wrapper behavior:
  - if global guard `byte_1059C7D4 != 0`, returns `0`
  - else delegates to `sub_100862F0(a3, lpbKeyState)`
- core chain:
  - `ImeProcessKey -> sub_100862F0 -> sub_1009B770`
- notes:
  - observed key/scancode normalization and keyboard state handling
  - interacts with message path (`SendMessageW`)

## 2) ImeToAsciiEx

- addr: `0x10085750`
- proto: `UINT __stdcall(UINT uVirtKey, UINT uScaCode, const LPBYTE lpbKeyState, LPTRANSMSGLIST lpTransBuf, UINT fuState, HIMC)`
- wrapper behavior:
  - if global guard `byte_1059C7D4 != 0`, returns `0`
  - else delegates to `sub_100865A0(lpbKeyState, lpTransBuf, fuState, HIMC)`
- core chain:
  - `ImeToAsciiEx -> sub_100865A0 -> sub_1009BC60`
- notes:
  - candidate/composition transition logic appears in deeper chain
  - explicit internal trace markers include `ImeToAsciiEx2`, `ImeToAsciiEx7_*`

## 3) NotifyIME

- addr: `0x10085820`
- proto: `BOOL __stdcall(HIMC, DWORD, DWORD, DWORD)`
- wrapper behavior:
  - if global guard `byte_1059C7D4 != 0`, returns `0`
  - else delegates to `sub_100868E0(a3, a4)`
- core chain:
  - `NotifyIME -> sub_100868E0 -> sub_1009C170`
- notes:
  - uses IMM context operations (`ImmLockIMC/IMCC`, `ImmUnlockIMC/IMCC`)

## 4) ImeSelect

- addr: `0x10085500`
- proto: `BOOL __stdcall(HIMC, BOOL)`
- wrapper behavior:
  - if guard set, return `0`
  - else delegates to `sub_10085FC0()`
- notes:
  - selection activation/deactivation path and state/log updates

## 5) ImeConfigure

- addr: `0x10085330`
- proto: `BOOL __stdcall(HKL, HWND, DWORD, LPVOID)`
- wrapper behavior:
  - if guard set, return `0`
  - else delegates to `sub_10085BE0(...)`
- recovered behavior in `sub_10085BE0`:
  - checks/locates `SGTool.exe`
  - launch intent uses arg `--appid=config`
  - fallback `MessageBoxW` when executable missing

## 6) ImeInquire

- addr: `0x10085260`
- proto: `BOOL __stdcall(LPIMEINFO lpIMEInfo, LPTSTR lpszUIClass, DWORD dwSystemInfoFlags)`
- wrapper behavior:
  - if guard set, return `0`
  - else delegates to `sub_10085930(dwSystemInfoFlags)`
- recovered behavior in `sub_10085930`:
  - initializes IME info fields
  - sets UI class text via `wcscpy_s(..., word_10599648)`

## 7) ImeEscape

- addr: `0x100853F0`
- proto: `LRESULT __stdcall(HIMC, UINT, LPVOID)`
- behavior:
  - returns `0` if guard set
  - only handles command `a2 == 4102`
  - writes static wide-string token to output buffer and returns `1`

## Bridge-facing implication

Minimal POC bridge should center around this order:

1. `ImeInquire` (capability/init metadata)
2. `ImeSelect` + `ImeSetActiveContext`
3. `ImeProcessKey` + `ImeToAsciiEx` loop
4. `NotifyIME` for state transitions

Configuration and advanced controls should be treated as non-blocking extension.
