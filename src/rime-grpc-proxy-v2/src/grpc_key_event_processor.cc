#include "grpc_key_event_processor.h"
#include <rime/config.h>
#include <rime/schema.h>

namespace rime {

GrpcKeyEventProcessor::GrpcKeyEventProcessor(const Ticket& ticket)
    : Processor(ticket) {
  std::string backend_address = "127.0.0.1:50051";
  int rpc_timeout_ms = 100;
  bool fallback_on_error = true;

  if (auto* config = ticket.engine->schema()->config()) {
    config->GetString("grpc_proxy/backend_address", &backend_address);
    config->GetInt("grpc_proxy/rpc_timeout_ms", &rpc_timeout_ms);
    config->GetBool("grpc_proxy/fallback_on_error", &fallback_on_error);
  }

  client_ = GrpcImeClientV2::GetOrCreate(backend_address, rpc_timeout_ms, fallback_on_error);
}

ProcessResult GrpcKeyEventProcessor::ProcessKeyEvent(const rime::KeyEvent& key_event) {
  return kNoop;
}

} // namespace rime
