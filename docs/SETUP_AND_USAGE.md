# Setup and Usage Guide

> How to build, configure, and run the Sogou IME gRPC Bridge.

---

## Prerequisites

### Linux Host

| Software | Version | Purpose |
|---|---|---|
| Rust toolchain | stable (≥ 1.78) | Build `ime-grpc-host-v2` |
| `i686-pc-windows-gnu` target | (via `rustup`) | Cross-compile 32-bit Windows binary (for QQPinyin, Sogou 10.5b) |
| `x86_64-pc-windows-gnu` target | (via `rustup`) | Cross-compile 64-bit Windows binary (for Sogou 16.3) |
| CMake | ≥ 3.16 | Build `rime-remote` plugin |
| GCC / Clang | C++17 capable | Build `rime-remote` plugin |
| gRPC C++ | ≥ 1.50 | gRPC library and `protoc` plugin |
| Protobuf | ≥ 3.21 | Proto compiler |
| librime | ≥ 1.9 | Rime engine (runtime + development headers) |
| Wine | ≥ 9.0 | Run Windows IME DLLs (32-bit and/or 64-bit prefix) |
| Xvfb | any | Only needed for running IME installers (not for the host process) |
| MinGW-w64 | i686 and/or x86_64 | Cross-linker for Windows targets |

### Windows IME DLL

You need a `.ime` file from one of:

| IME | Version | Arch | Installer |
|---|---|---|---|
| **QQPinyin** | 6.6.6304 | 32-bit | `QQPinyin_Setup_6.6.6304.400.exe` (NSIS, 7z-extractable) |
| **Sogou 10.5b** | 10.5.0.4737 | 32-bit | `sogou_pinyin_105b_xp.exe` (Inno Setup, silent install) |
| **Sogou 16.3** | 16.3.0.3318 | 64-bit | Community "green" (portable) package from wuyou.net; deployed via `a.bat` script (no Xvfb needed). WeType was considered but rejected — pure TSF/TIP, no IMM API. |

For 32-bit IMEs: place in Wine's `C:\Windows\system32\` inside a 32-bit prefix.
For 64-bit IMEs: place in Wine's `C:\Windows\system32\` inside a 64-bit prefix.

---

## Building

### 1. Build ime-grpc-host-v2 (Rust, cross-compiled for Wine)

```bash
# Install Rust Windows targets
rustup target add i686-pc-windows-gnu      # 32-bit (QQPinyin, Sogou 10.5b)
rustup target add x86_64-pc-windows-gnu    # 64-bit (Sogou 16.3)

# Ensure MinGW cross-linkers are available
# Debian/Ubuntu:
sudo apt install gcc-mingw-w64-i686 gcc-mingw-w64-x86-64

# Build 32-bit Windows binary (for QQPinyin / Sogou 10.5b)
cd src/ime-grpc-host-v2
cargo build --release --target i686-pc-windows-gnu
# Output: target/i686-pc-windows-gnu/release/ime-grpc-host-v2.exe

# Build 64-bit Windows binary (for Sogou 16.3)
cargo build --release --target x86_64-pc-windows-gnu
# Output: target/x86_64-pc-windows-gnu/release/ime-grpc-host-v2.exe
```

**Linux-native build** (for testing with `NativeRimeBackend`):

```bash
cargo build --release
# Output: target/release/ime-grpc-host-v2
```

### 2. Build rime-remote (C++ librime plugin)

**Standalone build** (links to system librime):

```bash
cd src/rime-remote

# gRPC and Protobuf must be findable via CMake or pkg-config
cmake -B build \
  -DCMAKE_BUILD_TYPE=Release

cmake --build build

# Output: build/librime-remote.so
```

**Integrated build** (as part of librime):

When building librime from source, place `rime-remote` in `plugins/` and it will be compiled as part of `librime.so`:

```bash
# In the librime source tree:
cd plugins/
ln -s /path/to/src/rime-remote .

# Then build librime normally:
cd ..
cmake -B build -DBUILD_MERGED_PLUGINS=ON
cmake --build build
```

The CMakeLists.txt detects the `rime_library` variable set by librime's build system and compiles as an object library instead of a shared library.

**Weasel (Windows) build:**

```powershell
# In the librime source tree (Weasel uses its own librime):
$env:RIME_PLUGINS="rime-remote"
build.bat release
# gRPC triplet must be x64-windows-static
# Copy remote.schema.yaml to build/bin/Release for testing
```

**Squirrel (macOS) build:**

```bash
# Build grpc via homebrew first: brew install grpc
# Symlink plugin into librime/plugins/
ln -s /path/to/src/rime-remote librime/plugins/rime-remote
# Build via librime Makefile
make -C librime
# Output: librime/dist/lib/rime-plugins/librime-remote.dylib
```

---

## Wine Environment Setup

### 1. Create Wine Prefix

```bash
# 32-bit prefix (for QQPinyin, Sogou 10.5b)
export WINEPREFIX=~/.win32
export WINEARCH=win32
winecfg    # Initialize prefix; close the dialog

# 64-bit prefix (for Sogou 16.3)
export WINEPREFIX=~/.wine64
winecfg    # Initialize prefix; close the dialog
```

### 2. Install IME DLL

Xvfb is needed **only for running installers** (some require a display):

```bash
# For Sogou 10.5b (Inno Setup, needs display):
Xvfb :99 -screen 0 1024x768x24 &
DISPLAY=:99 wine sogou_pinyin_105b_xp.exe
kill %1

# For QQPinyin (extract from NSIS installer):
7z x QQPinyin_Setup_6.6.6304.400.exe -o/tmp/qq
cp /tmp/qq/Files/QQPinyin.ime ~/.win32/drive_c/windows/system32/
```

The host process itself does **NOT** require Xvfb — it uses `HWND_MESSAGE` windows that work without a display server.

### 3. Suppress Wine Dialogs

```bash
export WINEDLLOVERRIDES="mscoree=d;mshtml=d"
export WINEDEBUG=-all
```

### 4. Wine Wrapper Script (recommended)

Create `run_host.sh`:

```bash
#!/bin/bash
export WINEPREFIX=~/.win32
export WINEARCH=win32
export WINEDEBUG=-all
export WINEDLLOVERRIDES="mscoree=d;mshtml=d"

wine /path/to/ime-grpc-host-v2.exe \
  --bind 127.0.0.1:50051 \
  --ime-path "C:\\windows\\system32\\QQPinyin.ime"
```

For 64-bit (Sogou 16.3):

```bash
#!/bin/bash
export WINEPREFIX=~/.wine64
export WINEDEBUG=-all
export WINEDLLOVERRIDES="mscoree=d;mshtml=d"

wine /path/to/ime-grpc-host-v2.exe \
  --bind 127.0.0.1:50056 \
  --ime-path "C:\\windows\\system32\\SogouPY.ime"
```

---

## Configuration

### ime-grpc-host-v2 CLI Options

| Flag | Env Variable | Default | Description |
|---|---|---|---|
| `--bind` | `GRPC_BIND_ADDR` | `127.0.0.1:50051` | gRPC listen address |
| `--ime-path` | `GRPC_IME_PATH` | `C:\Windows\system32\SogouPY.ime` | Path to `.ime` DLL (Wine path) |
| `--show-window` | `GRPC_SHOW_WINDOW` | `false` | Show hidden message window (debug) |
| `--session-timeout-sec` | `GRPC_SESSION_TIMEOUT_SEC` | `600` | Auto-destroy idle sessions (seconds) |
| `--disable-punct-fallback` | `GRPC_DISABLE_PUNCT_FALLBACK` | `false` | Skip punctuation fallback table |

### rime-remote Schema

Copy `remote.schema.yaml` to Rime user directory (e.g., `~/.local/share/rime/`).

Key settings in the schema:

```yaml
remote:
  backend_address: "127.0.0.1:50051"   # must match --bind
  rpc_timeout_ms: 1000                  # gRPC deadline in milliseconds
  v_mode_preedit_regex: "^v\\d"         # Sogou v-mode detection regex

menu:
  page_size: 7                          # candidates per page

ascii_composer:
  switch_key:
    Shift_L: commit_code    # Left Shift: commit raw input + toggle ASCII
    Shift_R: commit_code    # Right Shift: same
```

**Switch key actions:**
- `commit_text` — commit current composition translation
- `commit_code` — commit raw input string
- `noop` — discard composition silently

### librime Plugin Loading

Ensure `rime-remote` is discoverable by librime:
- **Standalone**: Copy `librime-remote.so` to librime's plugin directory (e.g., `/usr/lib/rime-plugins/`)
- **Integrated**: No extra step needed if built as a merged plugin

Then set the schema as default in `default.custom.yaml`:

```yaml
patch:
  schema_list:
    - schema: remote
```

---

## Running

### Start the Server

```bash
# Terminal 1: IME host (no Xvfb needed)
./run_host.sh
# Expected output:
#   Starting RimeService v2 at 127.0.0.1:50051
```

### Start Rime

```bash
# Terminal 3: Deploy Rime schema and restart
rime_deployer --build
# Then restart your Rime frontend (ibus-daemon, fcitx5, etc.)
```

### Verify End-to-End

1. Open any text editor
2. Activate the "Remote IME" schema in Rime
3. Type `nihao` — you should see pinyin composition and Chinese candidates from the backend IME
4. Press `1` to select the first candidate — `你好` should be committed

### Test Client (manual verification)

```bash
cd src/ime-grpc-host-v2
cargo run --bin test-client -- --bind 127.0.0.1:50051
```

---

## Production Deployment (Dedicated Server)

This section describes the actual production setup running on a dedicated Debian 13 (trixie) machine. The host is `ime` — a bare-metal or VM server with no desktop environment.

### OS and Wine Installation

```bash
# Debian 13.4 (trixie)
cat /etc/debian_version   # 13.4

# Install Wine staging from official WineHQ repository
sudo mkdir -pm755 /etc/apt/keyrings
wget -O - https://dl.winehq.org/wine-builds/winehq.key | \
  sudo gpg --dearmor -o /etc/apt/keyrings/winehq-archive.key -
sudo wget -NP /etc/apt/sources.list.d/ \
  https://dl.winehq.org/wine-builds/debian/dists/trixie/winehq-trixie.sources
sudo apt update
sudo apt install --install-recommends winehq-staging
sudo apt install winetricks
```

### Dedicated Service User

A dedicated `ime` user (uid 1001) runs the Wine host. No login shell needed in production, but `/bin/bash` is set for maintenance:

```bash
# Created as: useradd -r -m -s /bin/bash ime
# Verify:
grep ime /etc/passwd
# ime:x:1001:1001::/home/ime:/bin/bash
```

### Sogou 16.3 Green Package Installation

Sogou 16.3 is installed from a "green" (portable) package obtained from: `https://bbs.wuyou.net/forum.php?mod=viewthread&tid=430198`

This is NOT the official Sogou installer — it's a community-maintained green package. The original green package's provided scripts (`_绿化.bat` etc.) call many Windows-specific tools (VBS, mshta, COM automation) that fail under Wine, so a custom `a.bat` was written to replace them.

The package is deployed to a 64-bit Wine prefix:

```
/opt/ime/.wine64/drive_c/Program Files/Sogou/
├── 16.3.0.3318/           # Version directory (contains SGTool.exe, SogouTSF.dll, etc.)
│   ├── _Green/            # Green package payload
│   │   ├── SogouPY.ime    # 32-bit IME DLL
│   │   ├── SogouPY64.ime  # 64-bit IME DLL (copied to system32 as SogouPY.ime)
│   │   ├── SogouTSF.ime   # 32-bit TSF component
│   │   ├── SogouTSF64.ime # 64-bit TSF component
│   │   ├── Binmay.exe     # Binary patcher (for env.ini tweaks)
│   │   ├── NirCmd.exe     # Shortcut creator
│   │   └── SogouPY/       # Default user data
│   └── Scd/               # Bundled cell dictionaries (.scel)
├── a.bat                  # ← Custom deployment script (see below)
├── _绿化.bat              # Original green script (NOT used — too Windows-specific for Wine)
└── _卸载.bat              # Original uninstaller
```

#### Deployment Script (`a.bat`)

Custom script created to replace the original green package scripts that fail under Wine. Performs 5 steps:

1. **Copy IME DLLs** to Wine system directories — detects 64-bit via `CommonProgramW6432`: `SogouPY64.ime` → `system32\SogouPY.ime`, `SogouPY.ime` → `SysWOW64\` (WoW64 layer)
2. **Patch `env.ini`** — binary-patch via Binmay to disable candidate emoji and hide status bar
3. **Write registry entries** — IME registration under `HKLM\SOFTWARE\SogouInput`, then block ads, telemetry, red-dot notifications, AI chat, and user-experience reporting under `HKCU\SOFTWARE\SogouInput*`
4. **Register IME components** — `SGTool.exe --appid=install -i -w`, `regsvr32` for TSF DLLs, install bundled cell dictionaries (`.scel`)
5. **Create shortcuts** — desktop shortcuts via NirCmd, Shortcut.exe, or mshta VBS fallback

Full script content:

```batch
@echo off
:: Switch to script directory
cd /d "%~dp0"
Set "SGdir=%CD%"
Set "XD=%SGdir:&=^&%"

:: Find version folder and enter
Set SGver=&for /f "delims=" %%i in ('dir /a:d /o:d /b 2^>nul') do if exist "%%~i\Data\Runtime.ini" Set "SGver=%%~i"
cd "%SGver%" 1>nul 2>nul

echo [1/5] Killing old processes and copying core files...
taskkill /f /im SGTool.exe /t 1>nul 2>nul

:: Copy IME files based on architecture
if defined CommonProgramW6432 (
    Set "SW=HKLM\SOFTWARE\WOW6432Node"
    copy /v /y "_Green\SogouPY.ime" "%WinDir%\SysWOW64\" 1>nul 2>nul
    copy /v /y "_Green\SogouPY64.ime" "%WinDir%\System32\SogouPY.ime" 1>nul 2>nul
    copy /v /y "_Green\SogouTSF.ime" "%WinDir%\SysWOW64\" 1>nul 2>nul
    copy /v /y "_Green\SogouTSF64.ime" "%WinDir%\System32\SogouTSF.ime" 1>nul 2>nul
) else (
    Set "SW=HKLM\SOFTWARE"
    copy /v /y "_Green\SogouPY.ime" "%WinDir%\System32\" 1>nul 2>nul
    copy /v /y "_Green\SogouTSF.ime" "%WinDir%\System32\" 1>nul 2>nul
)

echo [2/5] Applying binary patches (env.ini)...
Set "SGData=%AppData%"
xCopy /c /e /h /r /y "_Green\SogouPY" "%SGData%\SogouPY\" 1>nul 2>nul

:: Binmay: Disable candidate emoji
_Green\Binmay.exe -s 45006D006F006A006900460069006C006C003D0030 -S "0fffffffffffffffffff0ffffffffffffffffffff0" -r 45006D006F006A006900460069006C006C003D0030 -U "%SGData%\SogouPY\env.ini" 1>nul 2>nul
:: Binmay: Hide status bar
_Green\Binmay.exe -s 53007400610074007500730041007000700065006100720061006E00630065003D0030 -S "0fffffffffffffffffffffff0ffffffffffffffffffffffffffffffffffffffffffff0" -r 53007400610074007500730041007000700065006100720061006E00630065003D0032 -U "%SGData%\SogouPY\env.ini" 1>nul 2>nul

echo [3/5] Writing registry (Ad-block, Telemetry, File Associations)...
reg add "%SW%\SogouInput" /f /ve /t REG_SZ /d "%SGdir%" 1>nul 2>nul
reg add "%SW%\SogouInput" /f /v "EnableSogouEudc" /t REG_DWORD /d "1" 1>nul 2>nul
reg add "%SW%\SogouInput" /f /v "PatchFlag" /t REG_DWORD /d "1" 1>nul 2>nul
reg add "%SW%\SogouInput" /f /v "Version" /t REG_SZ /d "%SGver%" 1>nul 2>nul

:: Core settings to block ads, pop-ups, and AI models
reg add "HKCU\SOFTWARE\SogouInput" /f /v "Recover" /t REG_DWORD /d 0 1>nul 2>nul
reg add "HKCU\SOFTWARE\SogouInput" /f /v "UpUseBT" /t REG_DWORD /d 0 1>nul 2>nul
reg add "HKCU\SOFTWARE\SogouInput" /f /v "SendUserExperience" /t REG_DWORD /d 0 1>nul 2>nul
reg add "HKCU\SOFTWARE\SogouInput.store.user" /f /v "EnableShowJumpSogou" /t REG_DWORD /d 0 1>nul 2>nul
reg add "HKCU\SOFTWARE\SogouInput.store.user" /f /v "Status_Pop_Switch" /t REG_DWORD /d 0 1>nul 2>nul
reg add "HKCU\SOFTWARE\SogouInput.store.user" /f /v "Status_Skin_Red" /t REG_DWORD /d 0 1>nul 2>nul
reg add "HKCU\SOFTWARE\SogouInput.store.user" /f /v "Systoast_Enable" /t REG_DWORD /d 0 1>nul 2>nul
reg add "HKCU\SOFTWARE\SogouInput.store.user\Large_Model\IChat" /f /v "Enable_Try_Launch" /t REG_DWORD /d 0 1>nul 2>nul
reg add "HKCU\SOFTWARE\SogouInput.store.user\Large_Model\Temp" /f /v "Enable_Tlar" /t REG_DWORD /d 0 1>nul 2>nul

:: File associations for dictionaries (.scel) and skins (.ssf)
reg add "HKLM\SOFTWARE\Classes\.scel" /f /ve /t REG_SZ /d "SogouCellDict" 1>nul 2>nul
reg add "HKLM\SOFTWARE\Classes\.ssf" /f /ve /t REG_SZ /d "SogouSkinFile" 1>nul 2>nul
reg add "HKLM\SOFTWARE\Classes\SogouCellDict\Shell\Open\Command" /f /ve /t REG_SZ /d "\"%XD%\%SGver%\SGTool.exe\" -line 0 -border --appid=scdreg -add \"%%1\"" 1>nul 2>nul
reg add "HKLM\SOFTWARE\Classes\SogouSkinFile\Shell\Open\Command" /f /ve /t REG_SZ /d "\"%XD%\%SGver%\SGTool.exe\" -line 0 -border --appid=skinreg -install -c \"%%1\"" 1>nul 2>nul

echo [4/5] Registering components (SGTool) and loading dictionaries...
SGTool.exe --appid=install -i -w
regsvr32 /s "%CD%\SogouTSF.dll" 1>nul 2>nul
regsvr32 /s "%WinDir%\System32\SogouTSF.ime" 1>nul 2>nul
if defined CommonProgramW6432 regsvr32 /s "%WinDir%\SysWOW64\SogouTSF.ime" 1>nul 2>nul

SGTool.exe --appid=scdreg -register "%CD%"
SGTool.exe --appid=skinreg -register "%CD%"
SGTool.exe --appid=ucfont -yahei -extb6 "%CD%"
SGTool.exe --appid=pinyinrepair /k
SGTool.exe --appid=scdreg -ConvV1toV2InstPath "%CD%"

:: Batch install local .scel files
for %%a in (Scd\*.scel) do (SGTool.exe --appid=scdreg -add "%%~a" -s)

SGTool.exe --appid=scdreg -CombScd "%CD%"
SGTool.exe --appid=scdreg -cdefault
SGTool.exe --appid=userpage -register_protocol
SGTool.exe --appid=dictconv

echo [5/5] Creating shortcuts...
Set "lnkPath=%USERPROFILE%\Desktop"

:: Extract and create shortcuts
if exist "_Green\NirCmd.exe" (
    "_Green\NirCmd.exe" shortcut "%SGdir%\%SGver%\SGTool.exe" "%lnkPath%" "SG_Config" "--appid=config" "%CD%\SGTool.exe" 0
    "_Green\NirCmd.exe" shortcut "%SGdir%\%SGver%\SGTool.exe" "%lnkPath%" "SG_IME_Mgr" "--appid=config /m" "%CD%\SGTool.exe" 0
    "_Green\NirCmd.exe" shortcut "%SGdir%\%SGver%\SGTool.exe" "%lnkPath%" "SG_Symbols" "--appid=exinput -cid=totalsym" "%CD%\SGTool.exe" 0
) else if exist "_Green\Shortcut.exe" (
    "_Green\Shortcut.exe" "%lnkPath%\SG_Config.lnk" "%SGdir%\%SGver%\SGTool.exe" "--appid=config" "%CD%\SGTool.exe" 0
    "_Green\Shortcut.exe" "%lnkPath%\SG_IME_Mgr.lnk" "%SGdir%\%SGver%\SGTool.exe" "--appid=config /m" "%CD%\SGTool.exe" 0
    "_Green\Shortcut.exe" "%lnkPath%\SG_Symbols.lnk" "%SGdir%\%SGver%\SGTool.exe" "--appid=exinput -cid=totalsym" "%CD%\SGTool.exe" 0
) else (
    mshta VBScript:Execute("Set a=CreateObject(""WScript.Shell""):Set b=a.CreateShortcut(""%lnkPath%\SG_Config.lnk""):b.TargetPath=""%SGdir%\%SGver%\SGTool.exe"":b.Arguments=""--appid=config"":b.Save:Close")
    mshta VBScript:Execute("Set a=CreateObject(""WScript.Shell""):Set b=a.CreateShortcut(""%lnkPath%\SG_Symbols.lnk""):b.TargetPath=""%SGdir%\%SGver%\SGTool.exe"":b.Arguments=""--appid=exinput -cid=totalsym"":b.Save:Close")
)

echo Setup Complete.
exit /b
```

Run inside Wine:

```bash
sudo -u ime bash -c '
  export WINEPREFIX=/opt/ime/.wine64
  export WINEDEBUG=-all
  cd "/opt/ime/.wine64/drive_c/Program Files/Sogou"
  wine cmd /c a.bat
'
```

After this, `SogouPY.ime` (64-bit PE32+) lives at `C:\Windows\system32\SogouPY.ime` inside the Wine prefix.

### Systemd Service

The host runs as a systemd service with strong sandboxing:

```ini
# /etc/systemd/system/ime64.service
[Unit]
Description=Wine IME Service
After=network.target

[Service]
Type=simple
User=ime
Group=ime

# Wine environment
Environment="WINEPREFIX=/opt/ime/.wine64"
Environment="WINEDEBUG=-all"
Environment="DISPLAY="
Environment="XDG_RUNTIME_DIR=/run/ime"
Environment="XDG_CACHE_HOME=/run/ime/.cache"

# Application config
Environment="GRPC_BIND_ADDR=127.0.0.1:50056"
Environment="GRPC_IME_PATH=C:\\Windows\\system32\\SogouPY.ime"
Environment="RUST_LOG=trace"

TimeoutStartSec=120
ExecStartPre=/usr/bin/wineboot -u
ExecStart=/usr/bin/wine /opt/ime/ime-grpc-host-v2-64.exe

# Graceful Wine shutdown sequence
ExecStop=-/usr/bin/wineboot -e
ExecStop=-/usr/bin/wineserver -w
ExecStop=-/usr/bin/wineserver -k

KillMode=control-group
TimeoutStopSec=10
SendSIGKILL=yes
Restart=on-failure
RestartSec=5

# Filesystem sandboxing
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
InaccessiblePaths=/mnt /media /run/user
ReadWritePaths=/opt/ime/.wine64
RuntimeDirectory=ime

[Install]
WantedBy=multi-user.target
```

Key design choices:
- **`DISPLAY=""`** — explicitly unset. The host uses `HWND_MESSAGE` windows; no display server needed at runtime.
- **`ExecStartPre=wineboot -u`** — initializes/updates Wine prefix on every start (handles Wine version upgrades).
- **Three-stage stop** — `wineboot -e` (graceful WM_CLOSE) → `wineserver -w` (wait for registry flush) → `wineserver -k` (force kill as insurance).
- **`ProtectSystem=strict`** — the entire filesystem is read-only except `ReadWritePaths`.
- **`RuntimeDirectory=ime`** — creates `/run/ime` automatically for XDG runtime.

### Network Isolation (iptables)

Sogou 16.3 attempts to phone home for telemetry, updates, and cloud candidates. The `ime` user is firewalled to localhost only:

```bash
# Block all NEW outbound connections from uid 1001 (ime)
sudo iptables -A OUTPUT -m owner --uid-owner 1001 \
  -m conntrack --ctstate NEW -j REJECT --reject-with icmp-port-unreachable

# Allow localhost (for gRPC listener)
sudo iptables -A OUTPUT -d 127.0.0.1/32 -m owner --uid-owner 1001 -j ACCEPT

# Persist across reboots
sudo apt install iptables-persistent
sudo netfilter-persistent save
```

Resulting rules:

```
*filter
:INPUT ACCEPT
:FORWARD ACCEPT
:OUTPUT ACCEPT
-A OUTPUT -m owner --uid-owner 1001 -m conntrack --ctstate NEW -j REJECT
-A OUTPUT -d 127.0.0.1/32 -m owner --uid-owner 1001 -j ACCEPT
COMMIT
```

This ensures:
- gRPC clients can connect to port 50056 from the network (INPUT ACCEPT)
- The Wine process can only talk to localhost (no Sogou telemetry/cloud/update traffic)
- Established connections (e.g., inbound gRPC) continue working (conntrack `NEW` filter only)

### Service Management

```bash
sudo systemctl daemon-reload
sudo systemctl enable ime64.service
sudo systemctl start ime64.service

# Check status
sudo systemctl status ime64.service
sudo journalctl -u ime64.service -f   # Follow logs (RUST_LOG=trace)
```

---

## Logging

### ime-grpc-host-v2

Uses `tracing` with `RUST_LOG` env:

```bash
RUST_LOG=debug wine ime-grpc-host-v2.exe --bind 127.0.0.1:50051 --ime-path ...
```

Log levels: `error`, `warn`, `info`, `debug`, `trace`

### rime-remote

Uses librime's built-in logging. Enable via Rime configuration or `RIME_LOG_DIR`/`RIME_LOG_LEVEL` environment variables.

---

## Running Tests

### Rust Integration Tests

```bash
cd src/ime-grpc-host-v2

# Run all tests (requires Windows or Wine environment with IME DLL)
cargo test --target i686-pc-windows-gnu

# Run specific test
cargo test --target i686-pc-windows-gnu test_nihao

# Tests include:
#   test_nihao         — basic pinyin input "nihao" → candidates
#   test_multi_commit  — multiple sequential commit cycles
#   test_uppercase     — ASCII passthrough for capital letters
#   test_punctuation   — Chinese punctuation mapping
#   test_mixed_23_59   — digits + punctuation sequence "23:59"
```

### Linux-only Tests

```bash
cargo test  # uses NativeRimeBackend, no Win32 dependency
```

---

## Troubleshooting

| Problem | Cause | Fix |
|---|---|---|
| Server starts but no candidates returned | IME DLL not loaded or wrong path | Check `--ime-path` points to valid `.ime` file in Wine filesystem |
| `ImeProcessKey` always returns FALSE | IME not activated | Verify `ImeSelect(himc, true)` succeeds; for Sogou, check gate byte |
| Wine config dialog pops up on every launch | Missing `WINEDLLOVERRIDES` | Set `WINEDLLOVERRIDES="mscoree=d;mshtml=d"` |
| `CreateWindowExW` fails | No display | This should NOT happen — host uses `HWND_MESSAGE`. If it does, start Xvfb: `Xvfb :99 &` + `DISPLAY=:99` |
| gRPC connection refused | Server not running or wrong address | Verify `--bind` matches `backend_address` in schema |
| Deadlock on key press | Win32 calls from wrong thread | Ensure `ChannelImmAdapter` is used (default in production) |
| Punctuation not converted | IME returns empty result for punctuation | Verify `--disable-punct-fallback` is NOT set |
| Sogou returns wrong code path | PE version mismatch (byte_3554) | Patch Wine's `kernel32.dll` PE version to 6.1.7601 (Win7 SP1) |
| 64-bit TRANSMSG parsing errors | Wrong struct size | Use platform-correct TRANSMSG (24 bytes on 64-bit, 12 on 32-bit) |
