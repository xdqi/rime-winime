# Phase 1 Host Architecture Draft (Addressing Wine IMM Gap)

Date: 2026-04-02

## Priority pivot (WINABC-first)

Current priority is to validate the Win32 IME host framework using `WINABC.IME` first, then migrate the same verified lifecycle to `SogouPY.ime`.

Why this is now the default:

1. `WINABC.IME` already shows positive framework signals in our host (`select=1/activate=1`, `KEY 41 -> process=1`).
2. This gives a stable baseline to separate framework issues from Sogou-specific behavior.
3. Unknown WINABC behavior can be clarified quickly with IDA (`ImeSelect` / `ImeProcessKey` / `ImeToAsciiEx` internal checks).

## Why this exists

Current evidence shows:

1. `SogouPY.ime` can be loaded in Wine and exports are callable.
2. Flow probe currently gets:
   - `ImmCreateContext -> valid`
   - `ImeSelect(TRUE) -> 0`
   - `ImeProcessKey(VK_A) -> 0`
   - `ImeToAsciiEx(VK_A) -> 0, uMsgCount=0`
   - `NotifyIME(...) -> 0`

Therefore we need a dedicated host process that reproduces the IMM lifecycle assumptions of the IME DLL, not just export invocation.

In parallel, `WINABC.IME` under the same host can reach non-zero `ImeProcessKey` for `KEY 41`, confirming the framework route is viable.

## Target architecture

- `fcitx5-front` (Linux side)
  - sends key/state/context events over TCP
  - receives preedit/candidate/commit outputs

- `ime-host-win32` (Wine side process)
  - owns hidden host window + message loop
  - owns/maintains HIMC lifecycle
  - performs call sequencing into `SogouPY.ime`
  - converts outputs to TCP protocol responses

Default operation policy:

- host runs in hidden-window mode by default (`visible=0`)
- visible window mode is allowed only as explicit debug option
- this prevents UI side effects from contaminating measurement runs

## Source-backed implementation map

1. Wine IMC selection and `ImeSelect` behavior
- `wine/dlls/imm32/imm.c`
- see `imc_select_ime` and `pImeSelect(..., TRUE/FALSE)` transitions

2. Wine host test pattern (window + focus + message pump)
- `wine/dlls/imm32/tests/imm32.c`
- `ProcessKey` test: window creation, focus, IME open, key loop

3. ReactOS default IME message forwarding path
- `reactos/win32ss/user/user32/windows/defwnd.c`
- forwards `WM_IME_*` to default IME window; handles composition result flow

4. ReactOS IME UI message handling entry
- `reactos/win32ss/user/user32/misc/imm.c`
- `ImeWndProc_common` handling for `WM_IME_SETCONTEXT/NOTIFY/COMPOSITION`

These references support the same conclusion: host loop and lifecycle are mandatory.

## NT5src deep references

For concrete ctfmon/imm32/conime source anchors and extracted implementation notes, see:

- `/opt/sogou/src/reverse/phase1-winabc-nt5src-reference.md`
- `/opt/sogou/src/reverse/phase1-qq-nt5src-imm-timeline.md`

## Required host capabilities

1. Window and message pump
- create hidden window
- run `GetMessage/TranslateMessage/DispatchMessage`
- handle focus-like transitions

2. IMC lifecycle
- create/destroy HIMC
- associate context to host window (`ImmAssociateContextEx`)
- maintain thread affinity consistency

3. Export call sequencing
- startup:
  - `ImeInquire`
  - `ImeSelect(TRUE)`
  - ensure context/window state is active before key tests
- per key event:
  - `ImeProcessKey`
  - `ImeToAsciiEx`
  - conditional `NotifyIME`
- shutdown:
  - `ImeSelect(FALSE)`
  - context cleanup

4. State and mode synchronization
- conversion mode changes
- composition start/update/end events
- candidate pagination and selection events

5. Fault tolerance
- watchdog for host thread deadlock
- process restart + state reset
- protocol-level timeout and retry

## Protocol direction (TCP)

- request types:
  - `Init`, `FocusIn`, `FocusOut`, `KeyDown`, `KeyUp`, `ModeSet`, `Reset`
- response/event types:
  - `Ack`, `Preedit`, `CandidateList`, `Commit`, `State`, `Error`
- mandatory metadata:
  - protocol version
  - capability bitmap
  - sequence id / correlation id

## Immediate next technical tasks

1. WINABC-first dynamic baseline (current state)
- run smoke by default against `C:\windows\syswow64\WINABC.IME`
- lock expected status pattern (`select/activate/open/ctxMatch`)
- key matrix now includes `PIPE`, `PAGEDOWN/PAGEUP`, `PICK`
- host now exposes readable primary candidates for `PIPE ni` (e.g. `你|泥|拟...`)
- candidate decoding path supports codepage override (`CP`) and packed-wide fallback
- host now exposes echo/preedit telemetry (`PREEDIT`, and `CAND` embeds `comp/read`)
- host now supports per-key lifecycle tracing (`TRACE` / `TRACEPIPE`) to mirror real IME request granularity

2. WINABC static-assisted refinement (IDA)
- inspect why some tail candidates still decode as replacement glyph (`�`)
- determine whether remaining candidate sub-lists are auxiliary/engine-private blobs
- pin down a strict "interactive list selection" rule for all IMEs

3. Sogou migration phase
- replay the same host command path (`ACTIVATE -> CP -> PIPE -> CAND`) against `SogouPY.ime`
- compare telemetry fields against WINABC baseline (`cand count/sel/page/compBytes`)
- current A/B snapshot: WINABC => readable primary CAND after `PIPE ni`; Sogou => `select=0/process=0/cand=0`
- `winedbg --gdb` return-sequence A/B (same three callsites) confirms both sides execute callsites per key, but only WINABC returns non-zero from `ImeProcessKey/ImeToAsciiEx` during `TRACE nihao`; Sogou stays zero on both.
- isolate Sogou-specific blockers only after this parity test

4. CONV path closure
- investigate why `CONV ni` remains `count=0` while `CAND` is non-empty
- align `ImeConversionList` source text and context with active composition state

## Success criteria

1. WINABC baseline is reproducible and non-zero on key processing.
2. Host lifecycle is documented as a portable contract (message/context/order).
3. Sogou tests run on the same contract without ad-hoc host changes.
