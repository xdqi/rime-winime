#include <rime/engine.h>
#include <rime/context.h>
#include <rime/processor.h>
#include "rime_service.grpc.pb.h"
#include "grpc_client.h"

using namespace rime::service::v2;

namespace rime {

class GrpcKeyEventProcessor : public Processor {
 public:
  GrpcKeyEventProcessor(const Ticket& ticket);

  ProcessResult ProcessKeyEvent(const KeyEvent& key_event) override;

 private:
  std::shared_ptr<GrpcImeClientV2> client_;
};

} // namespace rime
