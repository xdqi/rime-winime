#include "grpc_key_event_processor.h"

namespace rime {

GrpcKeyEventProcessor::GrpcKeyEventProcessor(const Ticket& ticket)
    : Processor(ticket) {
}

ProcessResult GrpcKeyEventProcessor::ProcessKeyEvent(const rime::KeyEvent& key_event) {
  return kNoop;
}

} // namespace rime
