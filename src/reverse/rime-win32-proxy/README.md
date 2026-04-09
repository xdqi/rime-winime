# rime-win32-proxy

A standalone external Rime plugin that exposes a Win32 IME backend as a Rime translator.

This project does not modify `arif` or `librime`. It only builds a plugin shared library (`rime-win32-proxy.so`) and a sample schema.

## What It Does

- Registers module: `win32_proxy`
- Registers component: `win32_proxy_translator`
- On each translator query, sends a proxy command to a running Win32 host service (for example `ime_host_skeleton.exe` under Wine)
- Parses `CAND_RET` and maps host candidates to Rime `SimpleCandidate`

## Runtime Requirements

- librime built/installed with external plugin loading support (`ENABLE_EXTERNAL_PLUGINS=ON`)
- A running host service compatible with the protocol in [src/reverse/host/README.md](../host/README.md)
- Host default endpoint: `127.0.0.1:22345`

## Build Requirements

- System package `librime-dev` (or equivalent development package that provides `pkg-config --libs rime`)
- `pkg-config`
- `cmake`
- C++17 compiler

## Build

```bash
cd /opt/sogou/src/reverse/rime-win32-proxy
cmake -S . -B build
cmake --build build -j
```

This project links against the system-installed librime (`pkg-config rime`) and does not use the local `/opt/sogou/librime` source tree for compilation.

## Install

```bash
cmake --install build
```

By default this installs the plugin to `${libdir}/rime-plugins`.

## Schema Example

Example schema is at [schema/win32_proxy.schema.yaml](schema/win32_proxy.schema.yaml).

Key config section:

```yaml
win32_proxy:
  host: 127.0.0.1
  port: 22345
  codepage: 936
  timeout_ms: 2500
  command: TEXTU
  tag: abc
  max_candidates: 9
  comment_mode: read
  one_shot: false
```

## Notes

- This is a translator-layer proxy core. Candidate selection is committed by Rime frontend behavior.
- If your host service endpoint differs, update `win32_proxy.host` and `win32_proxy.port` in schema config.
- For QQPinyin backend, `TEXTU` is the recommended command; `PIPEU` may clear composition and return no candidate items.

## Non-Invasive TAB Test (No Install)

You can run an end-to-end TAB smoke test without `cmake --install`.

This flow uses:

- local plugin build output `build/librime-win32-proxy.so`
- local arif build outputs under `/opt/sogou/arif/build/src/.libs`
- runtime injection via `LD_PRELOAD`
- expect automation to send TAB to a readline shell
- QQPinyin host backend (`C:\\windows\\system32\\QQPinyin.ime` by default)
- protocol-level readiness probe (`HELLO` + `PING/PONG`) before starting expect

Run:

```bash
cd /opt/sogou/src/reverse/rime-win32-proxy
./scripts/run_arif_tab_smoke_qqpinyin.sh
```

Optional overrides:

```bash
QQPY_HOST_PORT=22912 \
QQPY_DLL_PATH='C:\\windows\\system32\\QQPinyin.ime' \
QQPY_WINEPREFIX=/opt/sogou/.wine32 \
QQPY_INPUT_TEXT=ni \
./scripts/run_arif_tab_smoke_qqpinyin.sh
```

Run 3 rounds with different input text:

```bash
cd /opt/sogou/src/reverse/rime-win32-proxy
./scripts/run_arif_tab_smoke_qqpinyin_3inputs.sh
```

This now runs in a single process/session and simulates one-line edits with backspace transitions (example: ni<退格><退格>hao<退格><退格><退格>zhong).

Default inputs are `ni`, `hao`, and `zhong`. You can override all three with:

```bash
QQPY_INPUTS_CSV='ni,zhong,guo' ./scripts/run_arif_tab_smoke_qqpinyin_3inputs.sh
```
