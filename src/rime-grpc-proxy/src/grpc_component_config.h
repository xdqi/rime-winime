#ifndef RIME_GRPC_PROXY_COMPONENT_CONFIG_H_
#define RIME_GRPC_PROXY_COMPONENT_CONFIG_H_

#include <string>

#include <rime/common.h>
#include <rime/ticket.h>

namespace rime {
namespace grpc_proxy {

struct GrpcClientConfig {
  std::string host = "127.0.0.1";
  int port = 50051;
  int timeout_ms = 120;
  int max_candidates = 9;
  bool debug_stop_mode = false;
  std::string frontend_id = "rime-grpc-proxy";
  std::string schema_id;
};

GrpcClientConfig LoadGrpcClientConfig(const Ticket& ticket,
                                      const std::string& fallback_namespace);

}  // namespace grpc_proxy
}  // namespace rime

#endif  // RIME_GRPC_PROXY_COMPONENT_CONFIG_H_
