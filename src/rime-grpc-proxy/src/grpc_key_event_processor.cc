#include "grpc_key_event_processor.h"

#include "grpc_component_config.h"

namespace rime {
namespace grpc_proxy {

GrpcKeyEventProcessor::GrpcKeyEventProcessor(const Ticket& ticket)
    : Processor(ticket) {
  const auto cfg = LoadGrpcClientConfig(ticket, "grpc_proxy");
  client_ = GrpcImeClient::GetOrCreate(ticket.engine, cfg);
}

ProcessResult GrpcKeyEventProcessor::ProcessKeyEvent(const KeyEvent& key_event) {
  if (client_) {
    (void)client_->SendKeyEvent(key_event);
  }
  // Keep Rime's native processor chain behavior unchanged.
  return kNoop;
}

}  // namespace grpc_proxy
}  // namespace rime
