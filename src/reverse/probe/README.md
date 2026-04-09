# IME Probe (Wine Dynamic Check)

## Purpose

Minimal runtime probe for `SogouPY.ime` to validate reverse-first feasibility with concrete dynamic evidence.

## Source

- `ime_probe.c`
- `ime_flow_probe.c`

## Build

From `/opt/sogou/src/reverse/probe`:

```bash
i686-w64-mingw32-gcc ime_probe.c -o ime_probe.exe
i686-w64-mingw32-gcc ime_flow_probe.c -o ime_flow_probe.exe -limm32
```

Expected artifact:

- `ime_probe.exe` (PE32 console)

## Run

Default (`sys`):

```bash
bash /opt/sogou/winabc/wine_run.sh ./ime_probe.exe
```

Explicit `syswow64` path:

```bash
bash /opt/sogou/winabc/wine_run.sh ./ime_probe.exe "Z:\\opt\\sogou\\syswow64\\SogouPY.ime"
```

Flow probe:

```bash
bash /opt/sogou/winabc/wine_run.sh ./ime_flow_probe.exe
```

## Verified output highlights

Both paths returned the same key evidence:

- Export addresses resolved:
  - `ImeInquire : 10085260`
  - `ImeEscape : 100853f0`
  - `ImeSelect : 10085500`
  - `ImeProcessKey : 10085680`
  - `ImeToAsciiEx : 10085750`
  - `NotifyIME : 10085820`
- `ImeInquire ret=1`
- `UI class: SoPY_UI`
- `IMEINFO fdwProperty=0x001e0002`
- `IMEINFO fdwConversionCaps=0x00000488`
- `ImeEscape(4102) ret=1`

## Interpretation

This confirms:

1. Wine can load the target IME DLL directly.
2. Key IMM export surface is callable at runtime.
3. Static reverse findings (entry addresses/prototypes) match dynamic behavior.

This is sufficient evidence to continue with deeper runtime probing of key/composition flow.

## Flow probe result (framework-gap evidence)

Observed from `ime_flow_probe.exe`:

- `ImmCreateContext -> valid handle`
- `ImeSelect(TRUE) -> 0`
- `ImeProcessKey(VK_A) -> 0`
- `ImeToAsciiEx(VK_A) -> 0, uMsgCount=0`
- `NotifyIME(NI_COMPOSITIONSTR,CPS_COMPLETE) -> 0`

Interpretation:

1. DLL loading and basic export calls work.
2. But without full Windows IME host environment (window/input-context lifecycle, message pump, focus/context association), core processing path does not activate.
3. This supports the reverse-first conclusion that we must build a dedicated IME host layer instead of expecting Wine desktop integration to provide the full framework automatically.
