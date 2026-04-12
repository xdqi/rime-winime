# Sogou IME gRPC Bridge

Use Chinese IMEs (Sogou Pinyin, QQPinyin) as backends for [Rime](https://rime.im/) input method framework — across Linux, macOS (Squirrel), and Windows (Weasel).

## How It Works

```
┌─────────────────────┐    gRPC     ┌─────────────────────┐
│  Rime frontend      │◄──────────►│  ime-grpc-host-v2   │
│  + rime-remote      │  (proto3)   │  (Rust, under Wine) │
│  plugin (v3)        │             │                     │
│                     │             │  Loads SogouPY.ime  │
│  fcitx5 / ibus /    │             │  via Win32 IMM API  │
│  Squirrel / Weasel  │             └─────────────────────┘
└─────────────────────┘
```

A Rime plugin (`rime-remote`) forwards keystrokes over gRPC to a Rust server (`ime-grpc-host-v2`) that loads a Windows `.ime` DLL through the Win32 IMM API. The server runs under Wine on Linux/macOS or natively on Windows. Candidates flow back to Rime for display.

## Components

| Component | Language | Description |
|---|---|---|
| **rime-remote** | C++ | librime plugin (v3). Standard Processor/Segmentor/Translator pipeline. |
| **ime-grpc-host-v2** | Rust | gRPC server. Loads `.ime` DLLs via Win32 IMM DDI. 32-bit and 64-bit targets. |
| **grpc-contract-v2** | Protobuf | Service definition (`rime.service.v2`): `ProcessKey`, `GetContext`, `GetCommit`, etc. |
| **rime-grpc-proxy-v2** | C++ | Predecessor plugin (v2). Monolithic Rime C API hooking. Superseded by rime-remote. |

### Supported IME DLLs

| IME | Version | Architecture |
|---|---|---|
| QQPinyin | 6.6.6304 | 32-bit |
| Sogou Pinyin (10.5b) | 10.5.0.4737 | 32-bit |
| Sogou Pinyin (16.3) | 16.3.0.3318 | 64-bit |

### Supported Rime Frontends

- **fcitx5-rime** / **ibus-rime** (Linux)
- **Squirrel** (macOS)
- **Weasel** (Windows)

## Project Structure

```
sogou/
├── src/
│   ├── rime-remote/          # v3 librime plugin (production)
│   ├── rime-grpc-proxy-v2/   # v2 librime plugin (superseded)
│   ├── ime-grpc-host-v2/     # Rust gRPC server
│   ├── grpc-contract-v2/     # Protobuf service definition
│   ├── reverse/              # Reverse engineering notes and POC scripts
│   └── memories/             # Working notes from development
├── docs/                     # Technical documentation
│   ├── LLM_CONTEXT.md        # AI assistant context manual
│   ├── ARCHITECTURE.md       # System architecture
│   ├── SETUP_AND_USAGE.md    # Build, deploy, and run guide
│   ├── DEVELOPMENT_HISTORY.md# Timeline and decisions
│   ├── GRPC_PROTOCOL.md      # gRPC protocol reference
│   ├── REVERSE_ENGINEERING.md # Sogou DLL reverse engineering
│   └── WIN32_IMM_INTERNALS.md# Win32 IMM API internals
└── ...
```

## Quick Start

### Prerequisites

- Rust stable (≥ 1.78) with `i686-pc-windows-gnu` and/or `x86_64-pc-windows-gnu` targets
- CMake ≥ 3.16, C++17 compiler
- gRPC C++ and Protobuf
- librime (≥ 1.9)
- Wine (≥ 9.0)
- A Windows `.ime` DLL (see table above)

### Build

```bash
# Rust host (32-bit, for QQPinyin / Sogou 10.5b)
cd src/ime-grpc-host-v2
cargo build --release --target i686-pc-windows-gnu

# Rust host (64-bit, for Sogou 16.3)
cargo build --release --target x86_64-pc-windows-gnu

# Rime plugin
cd src/rime-remote
cmake -B build -DCMAKE_PREFIX_PATH=/path/to/librime
cmake --build build
```

### Run

```bash
# Terminal 1: Start the IME host under Wine
WINEPREFIX=~/.wine64 WINEDEBUG=-all \
  wine target/x86_64-pc-windows-gnu/release/ime-grpc-host-v2.exe \
  --bind 127.0.0.1:50056 --ime-path "C:\\windows\\system32\\SogouPY.ime"

# Terminal 2: Deploy Rime schema and restart your frontend
rime_deployer --build
```

Type `nihao` in any editor — you should see Chinese candidates from Sogou.

See [docs/SETUP_AND_USAGE.md](docs/SETUP_AND_USAGE.md) for full setup instructions including Wine prefix creation, IME installation, production systemd deployment, and troubleshooting.

## Documentation

| Document | Audience | Description |
|---|---|---|
| [LLM_CONTEXT.md](docs/LLM_CONTEXT.md) | AI assistants | Compact context manual for code generation |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Developers | Component design, data flow, thread model |
| [SETUP_AND_USAGE.md](docs/SETUP_AND_USAGE.md) | Operators | Build, configure, deploy, troubleshoot |
| [DEVELOPMENT_HISTORY.md](docs/DEVELOPMENT_HISTORY.md) | Anyone | 10-day timeline, decisions, evolution |
| [GRPC_PROTOCOL.md](docs/GRPC_PROTOCOL.md) | Developers | Protobuf message reference, RPC semantics |
| [REVERSE_ENGINEERING.md](docs/REVERSE_ENGINEERING.md) | Researchers | Sogou DLL internals, byte_3554, IDA analysis |
| [WIN32_IMM_INTERNALS.md](docs/WIN32_IMM_INTERNALS.md) | Developers | IMM DDI lifecycle, COMPOSITIONSTRING, Wine quirks |

## Version History

| Version | Component | Architecture | Status |
|---|---|---|---|
| v1 | `rime-grpc-proxy` + `ime-grpc-host` | Direct Rime C API loading | Abandoned |
| v2 | `rime-grpc-proxy-v2` + `ime-grpc-host-v2` | Monolithic API hooking | Superseded |
| v3 | `rime-remote` + `ime-grpc-host-v2` | Standard Rime pipeline | **Production** |

## License

[MIT](LICENSE)
