#ifndef RIME_GRPC_PROXY_TRANSLATOR_H_
#define RIME_GRPC_PROXY_TRANSLATOR_H_

#include <memory>

#include <rime/common.h>
#include <rime/translator.h>

#include "grpc_client.h"

namespace rime {
namespace grpc_proxy {

class GrpcProxyTranslator : public Translator {
 public:
  explicit GrpcProxyTranslator(const Ticket& ticket);

  an<Translation> Query(const string& input, const Segment& segment) override;

 private:
  std::shared_ptr<GrpcImeClient> client_;
  string tag_;
  int max_candidates_ = 9;
  string comment_mode_;
};

}  // namespace grpc_proxy
}  // namespace rime

#endif  // RIME_GRPC_PROXY_TRANSLATOR_H_
