# Phase 1 WINABC NT5src Reference Notes

Date: 2026-04-02

## Scope

Reference source tree: `nt5src/Source/XPSP1/NT/windows`

Goal: extract ctfmon/imm32/conime behaviors that can be reused in the current Wine host candidate-list work.

## High-value references

1. IMM key-to-message pipeline
- File: `/opt/sogou/nt5src/Source/XPSP1/NT/windows/core/ntuser/imm/input.c`
- Key points:
  - `ImmTranslateMessage` calls `ImeToAsciiEx` and converts returned `TRANSMSG` into posted/sent messages.
  - If `iNum` exceeds inline buffer, it consumes `hMsgBuf` from IMC.
  - Real path is not only direct export invocation; message posting is part of behavior.

2. Active-context notification ordering
- File: `/opt/sogou/nt5src/Source/XPSP1/NT/windows/core/ntuser/imm/context.c`
- Key points:
  - `ImmSetActiveContext` sends `WM_IME_SETCONTEXT`.
  - On activation, calls `NtUserNotifyIMEStatus(hWnd, dwOpenStatus, dwConversion)`.
  - Fallback sends `WM_IME_SETCONTEXT(FALSE)` to default IME window.

3. Candidate list storage and retrieval semantics
- File: `/opt/sogou/nt5src/Source/XPSP1/NT/windows/core/ntuser/imm/ctxtinfo.c`
- Key points:
  - `ImmGetCandidateListCount*` reads `hCandInfo` and returns required size plus list count.
  - `ImmGetCandidateList*` validates `dwIndex < dwCount` and copies/encodes list data.
  - `totalSize > 0` can coexist with `dwCount == 0` when candidate info block exists but no active list.

4. Console IME host message pump model
- Files:
  - `/opt/sogou/nt5src/Source/XPSP1/NT/windows/core/ntcon/conime/conime.c`
  - `/opt/sogou/nt5src/Source/XPSP1/NT/windows/core/ntcon/conime/consubs.c`
  - `/opt/sogou/nt5src/Source/XPSP1/NT/windows/core/ntcon/conime/imefull.c`
- Key points:
  - Dedicated hidden window + message loop receives `WM_IME_STARTCOMPOSITION`, `WM_IME_COMPOSITION`, `WM_IME_NOTIFY`, `WM_IME_SETCONTEXT`.
  - Explicit handling of `IMN_OPENCANDIDATE`, `IMN_CHANGECANDIDATE`, `IMN_CLOSECANDIDATE`.
  - Key path distinguishes IME-hotkey and IME-processed keys before forwarding to console.

5. ctfmon loader and TSF startup
- File: `/opt/sogou/nt5src/Source/XPSP1/NT/windows/advcore/ctf/cicload/cicload.cpp`
- Key points:
  - ctfmon boot path sets Run key, initializes TSF (`TF_InitSystem`), creates loader window.
  - Shows why TSF/CTF services can affect IMM behavior even for IME UI flows.

6. msctfime candidate/UI bridge behavior
- Files:
  - `/opt/sogou/nt5src/Source/XPSP1/NT/windows/advcore/ctf/msctfime/cic.cpp`
  - `/opt/sogou/nt5src/Source/XPSP1/NT/windows/advcore/ctf/msctfime/cmpevcb.cpp`
  - `/opt/sogou/nt5src/Source/XPSP1/NT/windows/advcore/ctf/msctfime/candpos.cpp`
- Key points:
  - Sends `WM_IME_NOTIFY/IMN_CHANGECANDIDATE` when candidate window state changes.
  - Tracks candidate window open/close through compartments and callbacks.
  - Candidate window position is computed from composition/caret geometry and `CANDIDATEFORM`.

## Direct actions for current host

1. Keep `WM_IME_SETCONTEXT` + open/conversion notifications aligned with `context.c` behavior.
2. Continue dual-path telemetry: direct IME exports + IMM path (`ImmProcessKey`/`ImmTranslateMessage`).
3. Add candidate polling for multiple indexes (not only index 0) when `dwCount > 1` is observed.
4. Add optional synthetic `WM_IME_NOTIFY` candidate-state hints for experiments.
5. Only trigger composition complete (`NotifyIME(..., CPS_COMPLETE)`) via explicit command, not every key.

## Current conime-style status (2026-04-02)

1. Host key path now applies char-first encoding before `ImeToAsciiEx` (matches `input.c` behavior pattern).
2. `TEXT ni` can produce non-zero composition bytes (`compBytes=2`) in runtime telemetry.
3. `ImmGetCandidateList*` still reports no active candidate list (`lists=0`, `count=0`) for WINABC in current sequence.
4. `ImeConversionList` command path is wired (`CONV`) but currently returns empty for `ni` in this environment.
5. Next focus should be WINABC-specific candidate trigger keys/mode transitions from `ImeToAsciiEx` internals.
