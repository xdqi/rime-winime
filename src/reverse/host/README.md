# ime_host_skeleton

Minimal Win32 host process (runs under Wine) for framework validation.

Current default target is `QQPinyin.ime`.

This binary is not a full IME bridge yet. It establishes the missing host lifecycle pieces:

- hidden host window
- message pump loop
- HIMC create/associate/reset lifecycle
- export binding (`ImeInquire`, `ImeSelect`, `ImeProcessKey`, `ImeToAsciiEx`, `NotifyIME`)
- TCP command loop for remote driving

## Files

- `ime_host_skeleton.c`
- `ime_host_skeleton.exe` (build output)
- `run_host_smoke.exp` (expect automation script)

## Build

From `/opt/sogou/src/reverse/host`:

```bash
i686-w64-mingw32-gcc ime_host_skeleton.c -o ime_host_skeleton.exe -lws2_32 -limm32
```

## Run

Initialize the dedicated Wine prefix once:

```bash
bash /opt/sogou/winabc/wine_run.sh --init
```

```bash
bash /opt/sogou/winabc/wine_run.sh ./ime_host_skeleton.exe --port 22345
```

Headless container run (recommended):

```bash
xvfb-run -a bash /opt/sogou/winabc/wine_run.sh ./ime_host_skeleton.exe --port 22345 --dll "C:\\windows\\system32\\QQPinyin.ime"
```

By default, host window is hidden (non-visible mode).
Use this only for debug if you explicitly want to see the window:

```bash
bash /opt/sogou/winabc/wine_run.sh ./ime_host_skeleton.exe --port 22345 --show-window
```

Optional DLL path override:

```bash
bash /opt/sogou/winabc/wine_run.sh ./ime_host_skeleton.exe --dll "Z:\\opt\\sogou\\sys\\SogouPY.ime" --port 22345
```

## One-shot expect automation (no manual input)

```bash
./run_host_smoke.exp
```

Headless container run (recommended):

```bash
xvfb-run -a ./run_host_smoke.exp
```

The script will automatically:

1. pick a free localhost port
2. start `ime_host_skeleton.exe` under Wine
3. connect over TCP
4. run `STATUS`, `ACTIVATE`, `CP 936`, `PIPEU ni`, `PREEDIT`, `CAND`, `PAGEDOWN`, `PICK 1`, `CONV ni`, `STATUS`, `QUIT`
5. wait for clean host exit

Optional args:

```bash
./run_host_smoke.exp --port 22360
./run_host_smoke.exp --dll "C:\\windows\\system32\\QQPinyin.ime"
./run_host_smoke.exp --dll "Z:\\opt\\sogou\\syswow64\\SogouPY.ime"
./run_host_smoke.exp --show-window
```

## Protocol (TCP)

Connect to `127.0.0.1:<port>`.

Commands:

- `PING`
- `STATUS`
- `CP [codepage]` (get/set candidate ANSI decode codepage, default `936`)
- `KEY <hex_vk>` (example: `KEY 41`)
- `CAND` (read current candidate list from active HIMC)
	- return payload now also includes echo fields: `compBytes/readBytes/comp/read`
- `PREEDIT` (read IME echo/preedit state: comp/read/result strings)
- `TRACE <ascii>` (per-key trace without conversion trigger; returns state after each key)
- `TRACEPIPE <ascii>` (same as `TRACE`, then appends a final space trigger step `SPC*`)
	- Unicode variants: `TRACEU <utf8>` and `TRACEPIPEU <utf8>`
	- each step includes `proc/imm/ascii/msgs` + preedit + candidate counters
- `PAGEDOWN` / `PAGEUP` (candidate list paging)
- `PICK <0..9>` (simulate numeric candidate selection key)
- `TEXT <ascii>` (inject ascii sequence, then return candidate snapshot)
- `PIPE <ascii>` (conime-style end-to-end: type text then trigger conversion with space)
- Unicode variants: `TEXTU <utf8>` and `PIPEU <utf8>`
- `CONV [utf8]` (call `ImeConversionList`; no arg means use current composition string)
- `COMMIT` (explicitly finalize current composition via `NotifyIME`)
- `RESET`
- `QUIT`

## Smoke test result (current)

Observed startup:

- host starts successfully
- `uiClass=` (WINABC reports empty UI class in this environment)
- `visible=0` (default hidden mode)
- `status select=1`

Observed command session (WINABC baseline):

- `PING -> PONG`
- `STATUS -> select=1 activate=1 ...`
- `CP 936 -> OK CP 936`
- `PIPEU ni -> CAND_RET cp=936 ... items=[你|泥|拟|...]`
- `PREEDIT -> PREEDIT_RET comp=[...] read=[...] ...`
- `CAND` output now carries both candidates and echo (`comp/read`), so empty-candidate phases are still observable
- `PAGEDOWN -> ... sel=9 pageSize=9 ...`
- `PICK 1 -> CAND_RET ... data=none` (composition finalized)

Per-key tracing example:

- `TRACEU nihao -> n{comp=[n]} ; i{comp=[ni]} ; ... ; o{comp=[nihao]}`
- `TRACEPIPEU nihao -> ... ; SPC*{comp=[你好o]}`

Interpretation:

- conime-style continuous key path is now working for candidate extraction
- for WINABC, primary candidate page now decodes to readable Chinese entries
- candidate probing uses W-list first (A fallback) with configurable multibyte fallback decode (`CP`)
- auxiliary non-interactive candidate blobs are filtered from default `CAND` output
- echo/preedit state is now queryable independently via `PREEDIT` (important when candidates are temporarily empty)

## Debugger A/B result (`winedbg --gdb`)

Using the same key stream (`TRACE nihao`) and the same three post-call breakpoints:

- `0x403696` (after `ImmProcessKey`)
- `0x40371d` (after `ImeProcessKey`)
- `0x40375c` (after `ImeToAsciiEx`)

Observed return-value sequence summary:

- WINABC: `imm_nz=0`, `imepk_nz=5`, `ascii_nz=5`
- Sogou: `imm_nz=0`, `imepk_nz=0`, `ascii_nz=0`

Conclusion: both IMEs are invoked through the same host callsites per key, but only WINABC returns non-zero from `ImeProcessKey`/`ImeToAsciiEx` on this path. The current Sogou blocker is return behavior, not missing call invocation.

## Force36c PoC (debug-only, reproducible)

Goal: prove that setting Sogou internal gate byte `t_dataPrivate+0x36c` unlocks key processing.

Files:

- `poc/run_force36c_poc.sh`
- `poc/force36c_probe.gdb`
- `poc/trace_client_force36c.py`
- `poc/trace_client_pagewalk.py` (candidate paging probe)

Run:

```bash
cd /opt/sogou/src/reverse/host/poc
./run_force36c_poc.sh
```

Run with an alternate client script:

```bash
cd /opt/sogou/src/reverse/host/poc
POC_CLIENT_SCRIPT=trace_client_pagewalk.py ./run_force36c_poc.sh
```

Run with an alternate gdb probe script (useful for long page-walk runs):

```bash
cd /opt/sogou/src/reverse/host/poc
POC_GDB_SCRIPT=force36c_detach_once.gdb POC_CLIENT_SCRIPT=trace_client_pagewalk.py ./run_force36c_poc.sh
```

Run with visible host window (disable Xvfb temporarily):

```bash
cd /opt/sogou/src/reverse/host/poc
POC_USE_XVFB=0 POC_SHOW_WINDOW=1 POC_GDB_SCRIPT=force36c_detach_once.gdb ./run_force36c_poc.sh
```

What this PoC does:

1. starts host under `gdbserver`
2. attaches `i686-w64-mingw32-gdb`
3. at callback path `sub_100F6080` read site, forces `obj+0x36c = 1`
4. runs `TRACEU nihao`, then Unicode trigger sequences (`TRACEPIPEU/PIPEU`) and candidate polling
5. checks processing gate + preedit visibility + candidate visibility

Current client behavior:

- `trace_client_force36c.py` uses Unicode-first command path by default (`TRACEU`, `TRACEPIPEU`, `PIPEU`)
- optional ASCII trigger fallback can be enabled with `POC_ALLOW_ASCII_FALLBACK=1`

Expected success signals:

- gdb log contains `FORCE_36C[...]` writes and final `FORCE36C_SUMMARY writes>0`
- client log contains `POC_PASS first_proc=2 ... candidate_count=...`
- trace payload shows incremental preedit (`n/ni/nih/niha/nihao`)
- candidate probe eventually returns non-empty `CAND_RET ... count>0 ... items=[...]`

Log outputs are saved under `/opt/sogou/.cache/` as:

- `poc_force36c_host_<timestamp>.log`
- `poc_force36c_gdb_<timestamp>.log`
- `poc_force36c_client_<timestamp>.log`
- `poc_force36c_cmd_<timestamp>.gdb`

Note: this is a debug PoC only. It demonstrates the blocker location but is not a production fix.

## Session record addendum (2026-04-03)

Detailed timeline and raw evidence are in:

- `poc/SESSION_2026-04-03.md`

### Proven gate and callback chain

Runtime tracing on Sogou path showed:

- callback dispatcher `sub_100F2660` is reached for each key
- callback sequence includes `0x10062430`, `0x10062150`, `0x100623E0`, `0x1007FB80`, `0x100801D0`, `0x100817B0`, `0x10080410`, `0x10063520`
- in natural path, all above callbacks returned `0`

Predicate-level evidence:

- `code=0x205` path (`sub_100801D0`) is short-circuited by `sub_100F6080`
- `sub_100F6080` resolves to vfunc `0x100ACD50`, which reads byte at `this+0x36c`
- sibling predicate `sub_100F5F50` resolves to vfunc `0x100ACD30`, which reads byte at `this+0x30`

### Setter/read probe summary

Observed in current host lifecycle:

- `0x100ACD60` (setter for `+0x36c`) hit count: `0`
- `0x100ACD40` (setter for `+0x30`) hit count: `0`
- `0x100AA530` (setter for `+0x31`) hit count: `6`
- read snapshots remained `b30=0`, `b36c=0`

This means the host path does initialize nearby state (`+0x31`) but never flips the gating byte (`+0x36c`).

### What was attempted and ruled out

The following natural-trigger attempts did not produce a hit on `sub_100F6110` (the write path for `+0x36c`):

- host-side `NotifyIME` action/index/value permutations
- host-side `ImmNotifyIME` variants
- `WM_IME_CONTROL` command scans
- NRAW parameter scans
- key-prefix scans before trace (`Caps`, `Shift`, `Ctrl`, `Alt`, `Space`, mixed combos)

### Current boundary and handoff

What is proven:

- forcing `obj+0x36c=1` is sufficient for `code=0x205 ret=3`
- forced run yields `POC_PASS first_proc=2` and visible preedit growth (`n -> ni -> nih -> niha -> nihao`)

What is still missing:

- non-injected lifecycle path that naturally invokes `sub_100F6110`

Recommended next caller set for condition alignment:

- `0x100519B0`
- `0x10060290`
- `0x1006BAB0`
- `0x10100AC0`
- `0x10065060`
