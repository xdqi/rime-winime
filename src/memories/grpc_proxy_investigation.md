# GRPC Proxy Investigation - Squirrel (macOS)

## Source Locations
- Plugin source: `/Users/user/Projects/squirrel/download/imm-x-rime/src/rime-grpc-proxy-v2/src/grpc_proxy_module.cc`
- Plugin binary: `/Users/user/Projects/squirrel/lib/rime-plugins/librime-grpc-proxy-v2.dylib` (769512 bytes)
- Module name: `grpc_proxy_v2` (registered via RIME_REGISTER_MODULE)

## Key Findings
- Plugin uses glog (LOG(INFO)) + custom syslog wrapper (GRPC_SYSLOG_INFO)
- Plugin loader from plugins_module.cc loads from rime-plugins directory
- Schema scan discovers grpc-enabled schemas at initialization
- Same grpc_proxy_module.cc works on both macOS (Squirrel) and Windows (Weasel)
- macOS requires stdbool ABI handling (rime_get_api_stdbool vs rime_get_api)
