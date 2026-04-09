# Phase 1 Core Semantics Draft

Date: 2026-04-02
Goal: Identify semantics of the internal functions behind key/compose/notify paths.

## Confirmed central dispatcher

`sub_10100AC0` (called by `sub_1009C170` / NotifyIME path)

Observed:
- large switch over notify/event code (`a2`)
- event-sensitive behavior for composition cancellation and state transition
- interacts with focus/window state (`GetFocus`) and internal context object methods
- references string `ImeContext::NotifyIME_CompositionStr`

Interpretation:
- this is a high-value semantic hub for composition lifecycle transitions.

## Core status helper

`sub_1009D3F0`

Observed:
- called by all three deep handlers (`sub_1009B770`, `sub_1009BC60`, `sub_1009C170`)
- computes a status code based on context/runtime flags
- returns table/index-derived value or fallback constant `9`

Interpretation:
- likely a mode/state selector used to branch processing strategy.

## IMC I/O gate functions

`sub_10097F20`

Observed:
- invokes `sub_100F04B0(..., mode=15)`
- success returns `1`
- failure logs and returns `0`

`sub_10098020`

Observed:
- invokes `sub_100F1400(...)`
- success returns `1`
- failure logs, resets guard (`dword_1059968C = 0`), returns `0`

Interpretation:
- these two functions form a validation/apply gate around IMC data flow.
- they appear in key, ascii, and notify deep paths.

## Practical reverse targets (next)

1. `sub_100F04B0`
- expected role: fetch/prepare IMC-derived core data into internal object.

2. `sub_100F1400`
- expected role: apply/commit transformed state back to context.

3. `sub_10107780`
- expected role: global/session runtime object provider.

4. `sub_100F62E0`
- expected role: context readiness/health check gate used repeatedly before processing.

## Bridge impact

For a functional bridge POC, these internals are more important than config decoding:

- Input acceptance gate: `sub_10097F20`
- State application gate: `sub_10098020`
- Event dispatcher: `sub_10100AC0`
- Runtime mode resolver: `sub_1009D3F0`

This confirms reverse-first is technically sound and aligned with project critical path.

## Dynamic probe evidence (2026-04-02)

A minimal Windows probe was built and executed under Wine:

- source: `src/reverse/probe/ime_probe.c`
- binary: `src/reverse/probe/ime_probe.exe`

Two runtime paths were tested:

1. `Z:\\opt\\sogou\\sys\\SogouPY.ime`
2. `Z:\\opt\\sogou\\syswow64\\SogouPY.ime`

Both produced consistent results:

- export addresses resolved exactly as static reverse expected
- `ImeInquire` returned success (`ret=1`)
- UI class returned `SoPY_UI`
- IMEINFO key fields returned stable values:
	- `fdwProperty=0x001e0002`
	- `fdwConversionCaps=0x00000488`
	- `fdwSentenceCaps=0x00000000`
- `ImeEscape(4102)` returned success (`ret=1`)

Practical meaning:

- reverse results are not only static; they are dynamically callable in Wine
- the core exported IMM surface is viable for bridge-layer runtime integration
- we can now move to next-level probing for key-to-candidate flow (`ImeProcessKey` / `ImeToAsciiEx` sequence)

## Framework gap validation (important)

Additional flow probe (`ime_flow_probe.exe`) attempted a minimal runtime sequence:

1. `ImmCreateContext`
2. `ImeSelect(TRUE)`
3. `ImeProcessKey(VK_A)`
4. `ImeToAsciiEx(VK_A)`
5. `NotifyIME(NI_COMPOSITIONSTR, CPS_COMPLETE, 0)`

Observed result:

- context creation succeeded, but all core processing calls returned `0`
- no transmsg output (`uMsgCount=0`)

Conclusion:

- Wine is sufficient for loading/calling exports, but not sufficient by itself to emulate the full Windows IME framework lifecycle used by this DLL.
- Your concern is confirmed: desktop Wine integration typically forwards input through Linux IME path, not a full IMM host behavior for this reverse target.

Engineering implication:

- bridge path should not assume "drop-in IME" behavior from Wine desktop.
- we need a dedicated Windows-side host component (message loop + HIMC lifecycle + association/focus events + notify sequencing) and expose it to fcitx5 via TCP.
