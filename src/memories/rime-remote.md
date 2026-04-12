# rime-remote plugin

- Location: `librime/plugins/rime-remote/`
- Zero API hook architecture: uses standard Processor/Segmentor/Translator pipeline
- Components: `remote_processor`, `remote_segmentor`, `remote_translator`
- Shared state via `RemoteStateRegistry` (global Engine* -> state map)
- Reuses `grpc_client.h/cc` from `rime-grpc-proxy-v2`
- Build: `$env:RIME_PLUGINS="rime-remote"; build.bat release` from librime dir
- Needs unstashed build dirs (lib, build, dist not renamed to *_x64)
- gRPC triplet must be x64-windows-static
- Test: copy remote.schema.yaml + default.yaml to build/bin/Release, run rime_api_console
- Backend address configured in schema yaml under `remote/backend_address`
