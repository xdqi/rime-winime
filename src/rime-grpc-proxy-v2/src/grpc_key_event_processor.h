#include <rime/engine.h>
#include <rime/processor.h>
#include "grpc_client.h"

namespace rime {

class GrpcKeyEventProcessor : public Processor {
 public:
  GrpcKeyEventProcessor(const Ticket& ticket);

  ProcessResult ProcessKeyEvent(const KeyEvent& key_event) override;

 private:
  std::shared_ptr<GrpcImeClientV2> client_;
};

} // namespace rime
