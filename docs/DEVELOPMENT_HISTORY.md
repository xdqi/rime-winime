# Development History

> Full chronological narrative of the Sogou IME gRPC Bridge project.
> Covers April 2–11, 2026 — ten days of intensive AI-assisted development.

---

## Timeline Overview

| Phase | Date | Focus | Key Outcome |
|---|---|---|---|
| 0 | Apr 2 (morning) | Reverse engineering Sogou IME | GO decision: IME DDI exports confirmed viable |
| 1 | Apr 2 (afternoon) | C prototype host + WINABC testing | First IME-driven preedit/candidate capture |
| 1b | Apr 2 (night)–Apr 3 | Sogou force-activation via GDB | `nihao→你好` from Sogou under Wine |
| 2 | Apr 3 | QQ Pinyin validation + first Rime plugin | End-to-end input via `rime-win32-proxy` |
| 3 | Apr 4 | gRPC v1 architecture (Rust + C++) | Full gRPC pipeline with acceptance tests |
| 4 | Apr 5 | v2 rewrite: Dictator Processor + modularization | Production-quality modular architecture |
| 5 | Apr 5–7 | Latency fix, candidate selection, configuration | Stable 10–20ms keystroke latency |
| 6 | Apr 7–8 | Sogou punctuation investigation + PE version fix | Root cause found and patched |
| 7 | Apr 8 | 64-bit host + Weasel integration | 32-bit/64-bit dual-target, Weasel TSF analysis |
| 8 | Apr 9–10 | Squirrel (macOS) integration | rime-grpc-proxy-v2 running on Squirrel |
| 9 | Apr 11 | rime-remote (v3): architectural refactor | Proper Rime pipeline; replaces v2 API hooking |

**Machines used:**
- **container** (Docker/devcontainer on Linux) — Phases 0–1b, reverse engineering work
- **vm** (Linux VM with Wine) — Phases 2–7, all gRPC and integration work
- **win** (Windows, Weasel) — Phases 7–9, Weasel integration + rime-remote development
- **mac** (macOS, Squirrel) — Phases 8–9, Squirrel integration

**Development methodology:** AI-assisted pair programming throughout, with the user providing architectural direction and the AI implementing. ~750+ file modifications logged across 4 machines.

---

## Phase 0: Reverse Engineering & Feasibility

**Apr 2, 06:21–08:08 · container**

The project began with static analysis of `SogouPY.ime`, a 32-bit Windows PE DLL. Binary comparison confirmed that `system32/` and `syswow64/` copies are identical (MD5 `7022bd95...`).

Seven mandatory IME DDI exports were mapped:

| Export | Purpose |
|---|---|
| `ImeInquire` | Capability query |
| `ImeSelect` | Activate/deactivate IME on a context |
| `ImeProcessKey` | Test whether IME will handle a key |
| `ImeToAsciiEx` | Process key and generate translation messages |
| `NotifyIME` | Receive notifications (candidate selection, composition changes) |
| `ImeConfigure` | UI configuration |
| `ImeEscape` | Private escape interface |

Internal call chain analysis (IDA Pro) traced `ImeProcessKey` → `sub_100862F0` (central dispatcher) → `sub_1009B770` (composition logic) and `ImeToAsciiEx` → `sub_100865A0` → `sub_1009BC60`.

Two dynamic probes were built: `ime_probe.c` (basic function call tracing) and `ime_flow_probe.c` (detailed data flow tracing). Both ran under Wine with MinGW cross-compilation.

**Critical discovery:** Wine's desktop mode cannot provide a full IMM lifecycle. Probe calls to `ImeProcessKey` returned 0 because no proper HWND + HIMC context existed. This ruled out simple DLL wrapping and established the requirement for a dedicated Win32 host with a message pump.

**User directive that shaped the architecture:**
> *"其实类似于conime更好，毕竟我们只要一个输入->候选词列表的转换"*
> ("Something like conime would be better — we just need an input→candidate list conversion")

This "conime-style" vision — a minimal host that converts keystrokes to candidates without implementing a full framework — guided all subsequent architectural decisions.

**Deliverables:** `phase0-reverse-feasibility.md`, `phase0-ime-export-contract.md`, `phase1-core-semantics.md`

---

## Phase 1: C Prototype Host

**Apr 2, 08:08–15:13 · container**

A C prototype (`ime_host_skeleton.c`) was built as a Win32 console application with:
- Hidden message-only window (`CreateWindowExW` with `HWND_MESSAGE`)
- Input method context (`ImmCreateContext`, `ImeSelect`)
- Win32 message pump (`GetMessage`/`TranslateMessage`/`DispatchMessage`)
- TCP command interface for remote control

Testing began with WINABC (Windows built-in English/Chinese IME) as a simpler target than Sogou.

The file grew explosively: **14KB → 51KB → 64KB** as features were added:
- `TRACE` command: per-key tracing of `ImeProcessKey` + `ImeToAsciiEx` results
- `PREEDIT`/`CAND` commands: composition string and candidate list queries
- `TRACEPIPE`: streaming mode for automated testing
- Automated test harness via `expect` scripts (`run_host_smoke.exp`)

**WINABC quirk discovered:** Candidate text appeared as "pseudo-Unicode" — GBK-encoded bytes stored in `WCHAR` positions. Required codepage-aware decoding to get readable Chinese text. Documented in `winabc_candidate_decode.md`.

**NT5 source as reference:** The user provided Microsoft NT5 source code (`ntuser/imm/input.c`) which established the canonical IMM processing path:

```
ImmProcessKey → ImmTranslateMessage → ImeToAsciiEx → ImmPostMessages
```

This became the authoritative reference throughout the project.

---

## Phase 1b: Sogou Force-Activation

**Apr 2, 22:00 – Apr 3, 06:00 · container**

When the host was tested with Sogou (`SogouPY.ime`), `ImeProcessKey` always returned 0 (rejected). Extensive GDB probing (~20 different scripts) traced the problem to an internal flag at `t_dataPrivate + 0x36c` — a "gate byte" that controlled activation. Under the host, it was always 0.

**Breakthrough:** Force-patching `+0x36c` to 1 via GDB made `ImeProcessKey` return 2 (accepted). The first `TRACE nihao` command produced a real preedit string from Sogou.

A reproducible proof-of-concept was built:
- `force36c_probe.gdb` — GDB script to patch the gate byte at runtime
- `run_force36c_poc.sh` — Shell wrapper
- `trace_client_force36c.py` — Automated verification client

Additional discoveries:
- Sogou requires `app/10.5.0.4737/` data files alongside the DLL
- Registry double-version path bug found and fixed
- `WINEDEBUG=+file` tracing revealed Sogou's file access patterns

Once a pure Win32 prefix was created with a proper Sogou installation (via official installer), the PoC passed without manual GDB intervention — the gate byte was set automatically during normal IME initialization.

**Milestone:** First successful `nihao → 你好` candidate from Sogou Pinyin under Wine.

---

## Phase 2: QQ Pinyin & First Rime Plugin

**Apr 3, 11:36–22:41 · vm**

Development moved to the VM environment. QQ Pinyin was deployed via 7z extraction (the NSIS installer did not support silent mode).

QQ Pinyin proved more cooperative than Sogou:
- `NIHAO_PASS` — basic pinyin input ✅
- `VMODE_PASS` — v-mode expression input ✅
- `UMODE_PASS` — Unicode input mode ✅

**Key decision:** QQ Pinyin became the primary target IME due to its more standard IMM behavior.

The first Rime plugin was built: `rime-win32-proxy`, a C++ plugin containing `win32_proxy_translator.cc` that loaded the IME DLL directly within the Rime process. This worked but had stability issues and tight coupling. The file grew from 9KB to 14KB.

User feedback highlighted the problem:
> *"你这是起了三个进程啊，不能在一个进程里面搞定吗？"*
> ("You're running three processes — can't you do it in one?")

This drove the decision toward a cleaner two-process architecture with gRPC separation.

---

## Phase 3: gRPC v1 Architecture

**Apr 4, 00:45–14:20 · vm**

The full gRPC v1 system was built in a single day:

- **`ime-grpc-host`** (Rust) — gRPC server with Win32 IMM backend. `main.rs` grew from 17KB to 74KB as all IMM logic was implemented as a single monolithic file.
- **`rime-grpc-proxy`** (C++) — librime plugin that relayed keys over gRPC
- **`grpc-contract`** — v1 protobuf with 7 RPC methods (`ime_proxy.proto`)

Comprehensive testing infrastructure was built:
- `qq_strict_regression.sh` — regression test suite
- `qq_strict_acceptance.sh` — acceptance criteria verification
- `qq_strict_stability_30.sh` — 30-iteration stability test
- `qq_phase_a_freeze_gate.sh` — freeze readiness gate

**Key technical challenge:** VK mapping. The Rime side sends XKB-style keysyms while the IME needs Win32 `VIRTUAL_KEY` codes. The discovery of `VkKeyScanExW` as the mapping function (documented in `ime-wine-findings.md`) solved this.

**Recognized problem:** The monolithic `win_imm.rs` (55KB) and `main.rs` (74KB) were unsustainable. User feedback:
> *"你不觉得当前rust代码已经单文件很长了吗，该拆了"*
> ("Don't you think the current Rust code is way too long for a single file? Time to split it.")

---

## Phase 4: v2 Rewrite — Dictator Processor

**Apr 5, 03:14–11:54 · vm**

Two architectural vision documents drove a complete rewrite:

### Dictator Processor (`plan-dictatorProcessor.prompt.md`)

The core insight: Rime should become a **dumb frontend** that delegates 100% of input processing to the remote IMM backend. No local composition logic, no local candidate generation — everything comes from the gRPC server. The Rime plugin is a "dictator" that controls the pipeline completely.

### Rime-in-Rime-out (`plan-rimeInRimeOut.prompt.md`)

The validation strategy: build a **native Rime backend** first (Linux librime FFI) that proves the gRPC transport is correct. Then swap in the Win32 IMM adapter. If native Rime → gRPC → native Rime works perfectly, any Win32 adapter bugs are isolated.

### Implementation

The v2 system was built with clean modular structure:

**`grpc-contract-v2`** — New proto (`rime_service.proto`) with 6 RPCs, designed around native Rime concepts:
- `OpenSession` / `DestroySession` — lifecycle
- `ProcessKey` — keystroke processing
- `GetContext` / `GetCommit` — state queries (pull-based)
- `SelectCandidateOnCurrentPage` — direct candidate selection

**`ime-grpc-host-v2`** — Modular Rust server:
- `backend/mod.rs` — async `RimeBackend` trait
- `backend/native.rs` — Linux Rime FFI backend (golden reference)
- `win_imm/mod.rs` — `ImmRimeAdapter` (extracted from monolithic `win_imm.rs`)
- `win_imm/session.rs` — `WinImmSession` (HWND + HIMC lifecycle)
- `win_imm/imm_ops.rs` — Low-level IMM FFI operations
- `win_imm/vk_map.rs` — Keysym → VK mapping
- `win_imm/channel_adapter.rs` — Thread isolation via oneshot channels

**`rime-grpc-proxy-v2`** (v2) — C++ librime plugin (API hooking approach):
- Single monolithic `grpc_proxy_module.cc` that hooks the Rime C API at function-pointer level
- `GrpcKeyEventProcessor` — stub processor returning `kNoop` for all keys
- `GrpcImeClientV2` — gRPC client (later shared with rime-remote)
- Config prefix: `grpc_proxy/`
- Limitation: ascii_composer never runs, stdbool ABI workaround needed for Squirrel (macOS)

Design decisions documented in `win_imm_plan.md`:
1. One global `ImeFunctions` struct (DLL loaded once)
2. Per-session HWND+HIMC
3. Rime keysym → VK conversion in the adapter
4. `pending_commit` field for buffer management
5. 11 specific test cases defined in advance
6. Dual-mode session: create-on-open, reuse-on-subsequent

---

## Phase 5: Latency, Selection, and Stabilization

**Apr 5–7 · vm**

### Latency Crisis

The tokio runtime was starved because `RimeBackend` trait methods were synchronous — they blocked the reactor thread. Symptoms: keystroke latency spikes up to hundreds of milliseconds.

**Solution:** All trait methods converted to `async fn` with `#[tonic::async_trait]`. Win32 calls isolated to a dedicated OS thread via `ChannelImmAdapter` using `tokio::sync::oneshot` channels. Result: **p50 ~2.5ms, p95 ~3.7ms, p99 ~6.6ms** per keystroke.

Documented in `latency_tuning.md`.

### Candidate Selection Bug

When a candidate was selected, the preedit text was being committed instead of the selected candidate text. Root cause: the proxy was reading `GCS_COMPSTR` (preedit) instead of `GCS_RESULTSTR` (committed result).

Additionally, `GCS_RESULTSTR` persisted across composition cycles. Fixed by using `TRANSMSG` analysis from `ImeToAsciiEx` — only read result when `WM_IME_COMPOSITION | GCS_RESULTSTR` flag is present, and consume via `.take()`.

Documented in `candidate_selection_fix.md`.

### Preedit Echo Problem

Composition string tracking required careful state management to avoid stale preedit echoes. `ImmGetCompositionString(GCS_COMPSTR)` returns the current composition, but after a commit it may not be cleared synchronously. Solution: clear preedit state in the adapter when a commit is detected.

Documented in `preedit_echo_notes.md`.

---

## Phase 6: Sogou Punctuation Investigation

**Apr 7–8 · vm**

The deepest single investigation of the project (~300+ lines in `sogou-punctuation-investigation.md`).

### Problem

Sogou Pinyin lost all Chinese punctuation. Typing `,` after accepting a candidate produced a literal ASCII comma instead of `，`. QQ Pinyin handled this correctly.

### Investigation

Exhaustive debugging over multiple sessions:
1. COMPOSITIONSTRING memory dump — empty for standalone punctuation
2. ANSI codepage test — not a UTF-16 vs ANSI issue
3. Message dispatch analysis — no `WM_IME_COMPOSITION` generated
4. Hidden data scan — no secret storage location
5. TRANSMSG buffer analysis — empty for punctuation keys

All dead ends.

### Root Cause (via IDA Pro)

Sogou has two code paths controlled by an internal flag `byte_3554`:

| Path | Condition | Behavior |
|---|---|---|
| **Win7** | `byte_3554 = 0` | Standard IMM: writes to COMPOSITIONSTRING, fills hMsgBuf |
| **Win8+** | `byte_3554 = 1` | Custom: `SendMessageW(hWnd, 0x8BB8, ...)` — bypasses COMPOSITIONSTRING entirely |

Sogou detects the Windows version via `max(kernel32.dll PE version, RtlGetNtVersionNumbers)`. Wine's `kernel32.dll` reports version 10.0, so the Win8+ path is always activated. In this path, punctuation is sent via a private window message (`0x8BB8`) that our host does not understand.

### Fix Attempts

1. **Runtime hook** (`retour::GenericDetour` on `GetFileVersionInfoSizeW` / `VerQueryValueW`) — worked to force version 6.1, but the Win7 path produced raw pinyin instead of Chinese characters (additional Sogou initialization issue)
2. **PE version patching** — used `rcedit` to change Wine's `kernel32.dll` PE version to `6.1.7601` (Windows 7 SP1). This was the "nuclear option" but it worked cleanly

### Result

After PE version patching:
- `nihao + Space → 你好` ✅
- `nihao + comma → 你好，` ✅
- Standalone comma → `，` ✅
- Standalone period → `。` ❌ (still failing — remaining edge case)

The punctuation fallback table (`punct_map.rs`) was added as a safety net for cases where the IME does not handle standalone punctuation.

---

## Phase 7: 64-bit Host and Weasel Integration

**Apr 8 · vm + win**

### 64-bit Target

With 32-bit QQPinyin working stably, attention turned to 64-bit Sogou 16.3. A parallel test setup was established:

| Configuration | 32-bit (reference) | 64-bit (new) |
|---|---|---|
| Wine prefix | `~/.wine32` | `~/.wine64` |
| Cargo target | `i686-pc-windows-gnu` | `x86_64-pc-windows-gnu` |
| Port | `:50051` | `:50056` |
| IME DLL | QQPinyin.ime | SogouPY.ime (from Sogou 16.3) |

Key 64-bit challenges:
- **TRANSMSG alignment**: 64-bit uses 24-byte TRANSMSG (4+4pad+8+8) vs 32-bit 12-byte. Initial parsing read garbage; fixed using official `windows` crate types.
- **`byte_3554` flag**: Comprehensive analysis documented in `byte3554_analysis.md`. Offset `0xDE2` in the 64-bit binary controls Win7 vs Win8+ dispatch path.
- **PE version spoofing**: `version_hook.rs` created with `retour::GenericDetour` on `GetFileVersionInfoSizeW`/`VerQueryValueW` to return version 6.1.7601 (Win7 SP1). Also `patch_pe_version.py` for binary patching of `kernel32.dll`.

### Weasel Integration

Development moved to the Windows machine for Weasel (TSF-based Rime frontend) integration:

- Deep analysis of Weasel's TSF TIP architecture documented in `weasel_tsf_hosting_analysis.md`
- COM activation flow: `DllGetClassObject` → `CClassFactory::CreateInstance` → `WeaselTSF::ActivateEx`
- WeaselServer **stack overflow crash** discovered in `rime!UserDictManager::UpgradeUserDict` (recursive call). Diagnosed with WinDbg.
- `rime-grpc-proxy-v2` deployed as librime plugin inside Weasel, compiled with `build.bat release`
- Key event analysis (`weasel_grpc_key_event_analysis.md`) identified fundamental limitation: `GrpcKeyEventProcessor` returns `kNoop` for all keys, preventing ascii_composer from working

### TIP Hosting Strategy

A hybrid approach was documented (`tip_hosting_strategy.md`): bypass system registry by loading DLL + `DllGetClassObject`, but use system `msctf.dll` `CoCreateInstance(CLSID_TF_ThreadMgr)` for COM services. Simulates an application with a hidden window and `ITfContext` sinks.

---

## Phase 8: Squirrel (macOS) Integration

**Apr 9–10 · mac**

The same v2 plugin was ported to Squirrel (macOS Rime frontend):

- Build script `build-rime-grpc-proxy-v2.sh` created for macOS, using Homebrew gRPC
- Plugin compiled as `librime-grpc-proxy-v2.dylib` and deployed to `squirrel/download/dist/lib/rime-plugins/`
- Integration into librime's Makefile for automated builds
- **Stdbool ABI challenge**: Squirrel uses `rime_get_api_stdbool()` instead of `rime_get_api()`. The v2 plugin needed a `RimeStatusT<BoolT>` template to handle both `Bool=int` and `Bool=bool` struct layouts.
- Logging struggles: `syslog_utils.h` added for macOS syslog output; conflicts with abseil logging resolved
- Swift modifications in `SquirrelInputController.swift` and `SquirrelApplicationDelegate.swift` for debugging

After intense iteration on Apr 10–11:
> *"终于可以用了"* (“Finally it works!”)

Code cleanup and git commits organized. `grpc_proxy_module.cc` had grown to ~50KB on macOS.

---

## Phase 9: rime-remote (v3) — Architectural Refactor

**Apr 11, 05:49 · win**

With v2 working on both Weasel and Squirrel, the user decided on a clean rename and architectural refactor:

> *"可以。不过新项目名就叫remote吧，名字简洁一点"*
> (“OK. But new project name should just be ‘remote’, keep it concise”)

### Key Architectural Changes

| Aspect | v2 (rime-grpc-proxy-v2) | v3 (rime-remote) |
|---|---|---|
| Integration | Hooks Rime C API at function-pointer level | Standard Processor/Segmentor/Translator pipeline |
| Components | 1 stub processor (`kNoop`) | 3 proper components in schema pipeline |
| State | Global `g_sessions` map by session ID | `RemoteStateRegistry` by `Engine*` pointer |
| ASCII mode | Broken (ascii_composer never runs) | Works: ascii_composer before remote_processor in pipeline |
| Stdbool ABI | Complex template workaround for Squirrel | Not needed (component-level, never touches raw RimeStatus) |
| Config prefix | `grpc_proxy/` | `remote/` |

Both v2 and v3 share the same `GrpcImeClientV2` gRPC client code and the same `grpc-contract-v2` protocol.

### Bug Fixes in v3

Documented in `rime-remote-bugs.md`:

1. **commit_code outputting spaces**: Synthetic input was spaces; fixed by using actual preedit text from backend
2. **Cursor always at end**: SimpleCandidate had preedit set; fixed by using `ctx->set_caret_pos()` instead
3. **SSH character displacement**: `Segmentation::Reset()` prefix-matching kept stale segments + gRPC timeout (200ms → 1000ms for network backends)
4. **Double OpenSession**: `.default` schema triggers first open; `HasSession()` guard added
5. **Backend schema_id**: Added `backend_schema_id` config option (default: `"luna_pinyin"`)

### Network Backend Testing

rime-remote was tested over SSH to a network backend at `127.0.0.1`, confirming the cross-network deployment model works. Test results: `nihao+Space→“你好” ✓`, cursor navigation works, `ssh+Space→“试试” ✓`.

---

## Key Architectural Decisions Log

| # | Decision | Rationale | Phase |
|---|---|---|---|
| 1 | Reverse-engineer first, not wrap-and-pray | Need to understand IME internals before building integration | 0 |
| 2 | Dedicated Win32 host process required | Wine desktop mode lacks full IMM lifecycle | 0 |
| 3 | conime-style: input→candidates, nothing more | Minimal scope, maximal reliability | 0 |
| 4 | QQ Pinyin as primary target | More standard IMM behavior than Sogou | 2 |
| 5 | gRPC for process separation | Clean boundary, language-agnostic, allows Wine isolation | 3 |
| 6 | C → Rust migration for the host | POC C code had bugs; Rust provides safety + modularity | 3 |
| 7 | "Dictator Processor" pattern | Rime as dumb frontend maximizes backend control | 4 |
| 8 | Native Rime backend for validation | Proves transport correctness before Win32 adapter | 4 |
| 9 | Single dedicated Win32 thread + channels | Thread affinity requirement; async bridge via oneshot | 4 |
| 10 | PE version patching for Sogou | Only way to activate Win7 code path under Wine | 6 |
| 11 | 64-bit host support | Sogou 16.3 requires x86_64 target; WeType was considered but rejected (pure TSF/TIP, no IMM) | 7 |
| 12 | rime-remote (v3) over v2 API hooking | Proper Rime pipeline fixes ascii_composer, stdbool, modularity | 9 |
| 13 | Network-transparent deployment | gRPC works cross-machine (tested over SSH to 127.0.0.1) | 9 |

---

## File Growth Highlights

| File | Phase | Start Size | End Size | Description |
|---|---|---|---|---|
| `ime_host_skeleton.c` | 0–1 | 14 KB | 64 KB | C prototype: grew through entire reverse engineering phase |
| `main.rs` (host v1) | 3 | 17 KB | 74 KB | Monolithic Rust host: all IMM logic in one file |
| `win_imm.rs` (v1) | 3 | 2 KB | 55 KB | Monolithic Win32 IMM backend |
| `win32_proxy_translator.cc` | 2 | 9 KB | 15 KB | Pre-gRPC direct-loading Rime plugin |
| `grpc_proxy_module.cc` | 4–9 | 672 B | ~50 KB | v2 C++ proxy module (grew most on macOS/Squirrel) |

All monolithic files were eventually modularized: Phase 4's v2 rewrite for the Rust host, Phase 9's v3 refactor for the Rime plugin.

---

## Machines and Environment

### container (Docker/devcontainer)

Used for Phases 0–1b. Provided a clean Linux environment with:
- MinGW i686 cross-compiler
- Wine for running 32-bit Windows binaries
- GDB (MinGW variant) for runtime probing
- Xvfb for running IME installers headless

### vm (Linux VM with Wine)

Used from Phase 2 through Phase 7. Provided:
- Full Wine installation with 32-bit and 64-bit prefixes
- QQ Pinyin 6.6 (32-bit) and Sogou 10.5b (32-bit) installed
- Sogou 16.3 (64-bit) in `.wine64` prefix
- Rust toolchain with both `i686-pc-windows-gnu` and `x86_64-pc-windows-gnu` targets
- librime development headers
- gRPC/Protobuf C++ libraries

### win (Windows)

Used from Phase 7 onward. Provided:
- Native Weasel installation
- librime build environment with `build.bat`
- `rime-grpc-proxy-v2` and `rime-remote` plugin development
- WinDbg for crash analysis

### mac (macOS)

Used for Phases 8–9. Provided:
- Squirrel installation
- Homebrew gRPC and Protobuf
- librime with Makefile-based build
- Xcode command-line tools

The progression container → vm → win+mac tracked the project's evolution from reverse engineering to multi-platform deployment.

---

## AI-Assisted Development Patterns

The project was developed through continuous AI pair programming. Several patterns emerged:

1. **User as architect, AI as implementer** — The user provided high-level direction and domain knowledge (e.g., references to NT5 source, conime model) while the AI wrote code and documentation.

2. **Rapid iteration** — The most common user prompts were `做吧` ("Do it") and `继续` ("Continue"), indicating trust in the AI's implementation with minimal intervention.

3. **Quality enforcement** — The user regularly caught bugs and enforced standards:
   - *"两个问题: 1.上屏时不应该把preedit也上屏 2.上屏应该上实际上屏词而不是候选词"* — caught the preedit/commit confusion bug
   - *"做了修改就做一下回归测试"* — demanded regression testing discipline
   - *"拒绝，你必须使用tool create_file或者edit_file来写文件"* — enforced proper tool usage

4. **Resource awareness** — The user set runtime bounds (*"卡了吧，以后前台任务不许超过2分钟"*) and cache location preferences (*"你别往/tmp写入临时文件"*).

5. **Knowledge injection** — The user provided external reference material (NT5 source, ReactOS source, Wine source, Microsoft docs) that the AI would not have had access to otherwise.
