#ifndef RIME_GRPC_PROXY_COMMIT_OBSERVER_H_
#define RIME_GRPC_PROXY_COMMIT_OBSERVER_H_

#include <memory>

#include <rime/common.h>
#include <rime/processor.h>

#include "grpc_client.h"

namespace rime {

class Context;

namespace grpc_proxy {

class GrpcCommitObserver : public Processor {
 public:
  explicit GrpcCommitObserver(const Ticket& ticket);
  ~GrpcCommitObserver() override;

  ProcessResult ProcessKeyEvent(const KeyEvent& key_event) override;

 private:
  void OnCommit(Context* ctx);

  std::shared_ptr<GrpcImeClient> client_;
  connection commit_connection_;
};

}  // namespace grpc_proxy
}  // namespace rime

#endif  // RIME_GRPC_PROXY_COMMIT_OBSERVER_H_
