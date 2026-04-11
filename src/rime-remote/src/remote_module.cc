// grpc/abseil headers first — see grpc_proxy_module.cc for rationale.
#include "grpc_client.h"

#include <rime/common.h>
#include <rime/component.h>
#include <rime/registry.h>
#include <rime_api.h>
#include "remote_processor.h"
#include "remote_segmentor.h"
#include "remote_translator.h"

using namespace rime;

static void rime_remote_initialize() {
  LOG(INFO) << "registering components from module 'remote'.";
  Registry& r = Registry::instance();
  r.Register("remote_processor", new Component<RemoteProcessor>);
  r.Register("remote_segmentor", new Component<RemoteSegmentor>);
  r.Register("remote_translator", new Component<RemoteTranslator>);
}

static void rime_remote_finalize() {
  GrpcImeClientV2::ShutdownAll();
  LOG(INFO) << "[remote] finalized.";
}

RIME_REGISTER_MODULE(remote)
