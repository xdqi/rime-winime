# Rime Frontend vs Windows IMM Key Mapping (Weasel reverse engineering)
Rime Frontend -> Windows IMM Driver (e.g., QQPinyin) mapping requires careful payload reconstruction.
Rime uses `ibus` style keycodes and modifiers.
Windows IMM's `ImeProcessKey` expects the raw OS structure:
1. `vKey`: Virtual key code (mapped from Ascii char using `VkKeyScanW(char)`)
2. `lpbKeyState`: 256-byte array, with `VK_SHIFT`, `VK_CONTROL`, `VK_MENU` matching Rime's `mask`.
3. `lParam` (lKeyData): Must contain a valid structure containing Scan Code (`MapVirtualKeyW`) shifted to `16..23` bits, and transition state/prev state (bits 30/31). If `lKeyData` is completely 0, `ImeProcessKey` often returns 0 (ignores the keystroke).

## Test IMM Validation
The isolated tester `test_imm` has been refactored to emit valid Rime/ibus keycodes (e.g. `0x6E` for 'n' instead of `0x4E` for 'N').
Because of the new `vk_map::rime_to_vk` translation layer inside the Host, feeding it exactly what `arify` passes now correctly triggers QQPinyin and accurately parses the candidate responses without altering the test logic itself, validating both the standalone host functionality as well as the translation map.

## Latency / Architecture Optimizations
- **Problem**: Arify and typical RIME frontends invoke `process_key`, followed immediately and repeatedly by `get_status` and `get_context`. When each of these forces a gRPC RPC call into Wine/Windows loopback + COM calls inside IMM, latency accumulates to 150-300ms per keystroke.
- **Solution**: Aggregated `get_context` and `get_commit` directly into the `ProcessKeyResponse` payload. A single gRPC roundtrip now processes the key and immediately fetches candidates, caching them in the C++ Proxy. Subsequent `MyGetContext` demands from the frontend take 0 IPC cost, resolving the stutter.
