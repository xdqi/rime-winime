#ifndef RIME_GRPC_PROXY_KEY_EVENT_PROCESSOR_H_
#define RIME_GRPC_PROXY_KEY_EVENT_PROCESSOR_H_

#include <memory>

#include <rime/processor.h>

#include "grpc_client.h"

namespace rime {
namespace grpc_proxy {

class GrpcKeyEventProcessor : public Processor {
 public:
  explicit GrpcKeyEventProcessor(const Ticket& ticket);

  ProcessResult ProcessKeyEvent(const KeyEvent& key_event) override;

 private:
  std::shared_ptr<GrpcImeClient> client_;
};

}  // namespace grpc_proxy
}  // namespace rime

#endif  // RIME_GRPC_PROXY_KEY_EVENT_PROCESSOR_H_
