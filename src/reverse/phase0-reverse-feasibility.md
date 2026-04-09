# Phase 0 Reverse Feasibility Report (SogouPY.ime)

Date: 2026-04-02
Scope: Prove whether Windows reverse engineering is viable as the first milestone.

## Executive conclusion

Reverse engineering is feasible and has already produced actionable evidence.

- GO for reverse-first strategy.
- No blocker found at exported IME API level.
- Real logic is in internal handlers reachable from stable IMM exports.
- `sys/SogouPY.ime` and `syswow64/SogouPY.ime` are effectively the same binary in this package (same hash), so we only need one primary reverse track now.

## Binary identity and architecture

Target A: `/opt/sogou/sys/SogouPY.ime`
Target B: `/opt/sogou/syswow64/SogouPY.ime`

Observed (both):
- arch: 32-bit PE image
- base: `0x10000000`
- image size: `0x694000`
- md5: `7022bd95ba0555b853a4a4d31d53ffb6`
- sha256: `8f183472662d00ddfebd1bcff8c7bac64fc97405b3d2648e5fc7487af7df11f1`

Implication:
- Keep one reverse map first, then validate runtime behavior under target Wine deployment.

## High-value exported IME APIs (confirmed)

- `0x10085680` `ImeProcessKey( HIMC, UINT, LPARAM, const LPBYTE )`
- `0x10085750` `ImeToAsciiEx( UINT, UINT, const LPBYTE, LPTRANSMSGLIST, UINT, HIMC )`
- `0x10085500` `ImeSelect( HIMC, BOOL )`
- `0x10085820` `NotifyIME( HIMC, DWORD, DWORD, DWORD )`
- `0x10085330` `ImeConfigure( HKL, HWND, DWORD, LPVOID )`
- `0x10085260` `ImeInquire( LPIMEINFO, LPTSTR, DWORD )`
- `0x100853F0` `ImeEscape( HIMC, UINT, LPVOID )`

## Core call-chain evidence

### Input path A (key preprocessing)

`ImeProcessKey (0x10085680)`
-> `sub_100862F0`
-> `sub_1009B770`

Notable behavior seen in decompilation:
- state guards and function-state instrumentation markers
- key/scancode normalization
- keyboard state reads (`GetKeyboardState`)
- message interaction (`SendMessageW`)
- internal core dispatch and result handling

### Input path B (composition and candidate transition)

`ImeToAsciiEx (0x10085750)`
-> `sub_100865A0`
-> `sub_1009BC60`

Notable behavior seen in decompilation:
- explicit tracing marker strings like `ImeToAsciiEx2/3/6/7`
- broker/process-awareness check (`sogouimebroker.exe` string path)
- internal conversion pipeline with candidate/composition state transitions
- message interactions and IMC data flow

### Notification path

`NotifyIME (0x10085820)`
-> `sub_100868E0`
-> `sub_1009C170`

Notable behavior seen in decompilation:
- explicit IMM context operations: `ImmLockIMC`, `ImmLockIMCC`, `ImmUnlockIMC`, `ImmUnlockIMCC`
- message bridge events and core notify handling

## Configuration-related reverse evidence (non-blocking for P0)

`ImeConfigure (0x10085330)`
-> `sub_10085BE0`

Observed:
- looks for `SGTool.exe`
- launches config flow via parameter `--appid=config`
- fallback to `MessageBoxW` on missing target executable

Implication:
- Windows settings entry is present and hookable.
- This supports later fallback UX, but should not block reverse-first input milestone.

## Dependency reality (important for later runtime bridge)

Imports confirm strong Windows IMM and system coupling:
- IMM32: `ImmNotifyIME`, `ImmAssociateContextEx`, `ImmGenerateMessage`, `ImmLockIMC/IMCC`, ...
- COM/OLE: `CoInitialize`, `CoCreateInstance`, `CoUninitialize`
- IPC/process primitives: named pipe family (`WaitNamedPipeW`, `TransactNamedPipe`, `PeekNamedPipe`), thread/process APIs
- Networking exists (WinHTTP/WinINet) but not required for first input POC

Interpretation:
- Reverse is feasible.
- Runtime bridge will need careful IMM-compatible adaptation in Wine host.

## What is already proven

1. Stable exported IME surface exists and was resolved.
2. Export wrappers were decompiled and traced to non-trivial internal handlers.
3. Internal handlers for key path and ascii/conversion path were recovered with meaningful logic.
4. Configure path contains concrete command argument evidence (`--appid=config`).
5. `sys`/`syswow64` binary identity is same in this dataset, reducing reverse split cost.
6. Wine runtime probe can load IME and call key exports successfully (`ImeInquire`, `ImeEscape`).

## Dynamic proof snapshot (Wine)

Probe: `src/reverse/probe/ime_probe.exe`

Observed on both `sys` and `syswow64` paths:

- `ImeInquire ret=1`
- UI class: `SoPY_UI`
- `fdwProperty=0x001e0002`
- `fdwConversionCaps=0x00000488`
- `ImeEscape(4102) ret=1`

This converts the feasibility claim from static-only to static+dynamic evidence.

Important boundary:

- export-call success does **not** mean full Windows IMM framework compatibility in Wine desktop mode.
- a deeper flow probe showed core input path calls can still return zero without proper IME host lifecycle.

## Remaining to prove for Phase 0 completion

1. Recover minimal semantic contract for these four internals:
- `sub_1009B770`
- `sub_1009BC60`
- `sub_1009C170`
- `sub_10085BE0`

2. Build parameter/return behavior table for:
- `ImeProcessKey`
- `ImeToAsciiEx`
- `NotifyIME`
- `ImeSelect`

3. Validate one dynamic hypothesis set in Wine test harness:
- key event enters `ImeProcessKey`
- transition into `ImeToAsciiEx`
- candidate/commit observable through transmsg-like output path

## Next execution plan (immediate)

1. Deep-type pass on the four internal handlers (arguments, structs, flags).
2. Extract branch conditions that gate candidate generation and commit.
3. Build a minimal call-contract document for bridge implementation.
4. Start a tiny runtime probe harness to invoke the export sequence under Wine.

## Go/No-Go

Status: GO

Reason:
- No structural reverse blocker encountered.
- Core input functions are reachable and analyzable.
- Evidence quality is enough to proceed into Phase 1/2 reverse tasks immediately.
- Runtime integration must include a dedicated host layer; cannot rely on Wine desktop input integration alone.
