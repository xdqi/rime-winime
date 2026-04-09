#ifndef RIME_GRPC_PROXY_CLIENT_H_
#define RIME_GRPC_PROXY_CLIENT_H_

#include <cstdint>
#include <memory>
#include <string>
#include <vector>

#include <rime/key_event.h>

#include "grpc_component_config.h"

namespace rime {

class Engine;

namespace grpc_proxy {

struct CandidateView {
  std::string text;
  std::string comment;
  double quality = 0.0;
};

struct QueryResult {
  std::string composition;
  std::string reading;
  std::vector<CandidateView> candidates;
  int selected_index = 0;
  int page_size = 0;
};

class GrpcImeClient {
 public:
  static std::shared_ptr<GrpcImeClient> GetOrCreate(Engine* engine,
                                                     const GrpcClientConfig& cfg);

  explicit GrpcImeClient(const GrpcClientConfig& cfg);
  ~GrpcImeClient();

  bool SendKeyEvent(const KeyEvent& key_event);
  bool QueryCandidates(const std::string& input,
                       int max_candidates,
                       QueryResult* out);
  bool CommitSelection(const std::string& committed_text,
                       int candidate_index,
                       uint64_t seq_hint = 0);

 private:
  struct Impl;
  std::unique_ptr<Impl> impl_;

  bool EnsureSessionLocked();
  bool HandleStatusLocked(const char* callsite,
                          const std::string& session_id,
                          const std::string& error_code,
                          const std::string& error_message,
                          bool ok_transport);

  bool QueryCandidatesLocked(const std::string& input,
                             int max_candidates,
                             QueryResult* out,
                             bool allow_cached);
};

}  // namespace grpc_proxy
}  // namespace rime

#endif  // RIME_GRPC_PROXY_CLIENT_H_
