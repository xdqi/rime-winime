#include <rime/component.h>
#include <rime/registry.h>
#include <rime_api.h>

#include "grpc_commit_observer.h"
#include "grpc_key_event_processor.h"
#include "grpc_proxy_translator.h"

using namespace rime;

static void rime_grpc_proxy_initialize() {
  Registry& r = Registry::instance();
  r.Register("grpc_key_event_processor",
             new Component<grpc_proxy::GrpcKeyEventProcessor>);
  r.Register("grpc_proxy_translator",
             new Component<grpc_proxy::GrpcProxyTranslator>);
  r.Register("grpc_commit_observer",
             new Component<grpc_proxy::GrpcCommitObserver>);
}

static void rime_grpc_proxy_finalize() {}

RIME_REGISTER_MODULE(grpc_proxy)
