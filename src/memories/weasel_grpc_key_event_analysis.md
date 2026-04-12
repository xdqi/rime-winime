# Weasel gRPC Proxy Key Event Handling Analysis

## Key Event Flow
1. WeaselTSF (Text Services Framework) intercepts raw Windows keyboard events
2. KeyEventSink.cpp converts Windows WPARAM/LPARAM to weasel::KeyEvent (IBus keycodes + modifiers)
3. WeaselIPC Client sends via pipe to WeaselIPCServer
4. RimeWithWeaselHandler::ProcessKeyEvent() calls rime_api->process_key()
5. Rime Engine processes key through processors chain (ascii_composer first in normal Rime)

## Problem in rime-grpc-proxy-v2
- GrpcKeyEventProcessor::ProcessKeyEvent() returns kNoop for ALL keys
- ascii_composer never runs (not in processor list of grpc_proxy.schema.yaml)
- Shift key toggling completely disabled
- RPC protocol only returns accepted boolean — no key bindings or ascii_mode toggle support

## Solution (implemented in rime-remote)
- rime-remote adds ascii_composer BEFORE remote_processor in schema pipeline
- Shift toggling handled locally (never sent to gRPC backend)
- remote_processor handles all other keys via gRPC
- ASCII mode tracked in RemoteSharedState with configurable switch styles
