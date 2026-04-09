#include "grpc_commit_observer.h"

#include <rime/context.h>
#include <rime/engine.h>

#include "grpc_component_config.h"

namespace rime {
namespace grpc_proxy {

GrpcCommitObserver::GrpcCommitObserver(const Ticket& ticket)
    : Processor(ticket) {
  const auto cfg = LoadGrpcClientConfig(ticket, "grpc_proxy");
  client_ = GrpcImeClient::GetOrCreate(ticket.engine, cfg);

  if (engine_ && engine_->context()) {
    commit_connection_ =
        engine_->context()->commit_notifier().connect(
            [this](Context* ctx) { OnCommit(ctx); });
  }
}

GrpcCommitObserver::~GrpcCommitObserver() {
  commit_connection_.disconnect();
}

ProcessResult GrpcCommitObserver::ProcessKeyEvent(const KeyEvent& /*key_event*/) {
  return kNoop;
}

void GrpcCommitObserver::OnCommit(Context* ctx) {
  if (!ctx || !client_) {
    return;
  }

  const string commit_text = ctx->GetCommitText();
  if (commit_text.empty()) {
    return;
  }

  (void)client_->CommitSelection(commit_text, -1, 0);
}

}  // namespace grpc_proxy
}  // namespace rime
