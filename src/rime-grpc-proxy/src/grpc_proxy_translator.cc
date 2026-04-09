#include "grpc_proxy_translator.h"

#include <algorithm>

#include <rime/candidate.h>
#include <rime/config.h>
#include <rime/schema.h>
#include <rime/segmentation.h>
#include <rime/translation.h>

#include "grpc_component_config.h"

namespace rime {
namespace grpc_proxy {

GrpcProxyTranslator::GrpcProxyTranslator(const Ticket& ticket)
    : Translator(ticket) {
  const auto cfg = LoadGrpcClientConfig(ticket, "grpc_proxy");
  client_ = GrpcImeClient::GetOrCreate(ticket.engine, cfg);
  max_candidates_ = cfg.max_candidates;
  comment_mode_ = "read";

  Config* config = ticket.schema ? ticket.schema->config() : nullptr;
  if (config) {
    const std::string ns = ticket.name_space.empty() ? "grpc_proxy"
                                                      : ticket.name_space;
    (void)config->GetString(ns + "/tag", &tag_);
    (void)config->GetString(ns + "/comment_mode", &comment_mode_);
  }
}

an<Translation> GrpcProxyTranslator::Query(const string& input,
                                           const Segment& segment) {
  if (input.empty() || !client_) {
    return nullptr;
  }

  if (!tag_.empty() && !segment.HasTag(tag_)) {
    return nullptr;
  }

  QueryResult result;
  if (!client_->QueryCandidates(input, max_candidates_, &result)) {
    return nullptr;
  }

  if (result.candidates.empty()) {
    return nullptr;
  }

  an<FifoTranslation> translation = New<FifoTranslation>();
  const size_t count = std::min(
      result.candidates.size(), static_cast<size_t>(std::max(1, max_candidates_)));

  for (size_t i = 0; i < count; ++i) {
    const auto& item = result.candidates[i];

    string comment;
    if (comment_mode_ == "read") {
      comment = result.reading;
    } else if (comment_mode_ == "static") {
      comment = "grpc";
    } else {
      comment = item.comment;
    }

    const string preedit =
        result.composition.empty() ? input : result.composition;

    auto cand = New<SimpleCandidate>("grpc_proxy", segment.start, segment.end,
                                     item.text, comment, preedit);
    cand->set_quality(item.quality > 0.0 ? item.quality : (100.0 - i));
    translation->Append(cand);
  }

  return translation;
}

}  // namespace grpc_proxy
}  // namespace rime
