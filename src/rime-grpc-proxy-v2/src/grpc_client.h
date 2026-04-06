#ifndef RIME_GRPC_CLIENT_V2_H_
#define RIME_GRPC_CLIENT_V2_H_

#include <string>
#include <memory>
#include <optional>
#include <regex>
#include <grpcpp/grpcpp.h>
#include <mutex>
#include <unordered_map>
#include "rime_service.grpc.pb.h"
#include <rime/key_event.h>
#include <rime/engine.h>

namespace rime {

class GrpcImeClientV2 {
public:
  static std::shared_ptr<GrpcImeClientV2> Instance();
  static std::shared_ptr<GrpcImeClientV2> GetOrCreate(const std::string& target_address, int timeout_ms, bool fallback_on_error);
  
  GrpcImeClientV2(const std::string& target_address, int timeout_ms, bool fallback_on_error);
  ~GrpcImeClientV2();

  bool FallbackOnError() const { return fallback_on_error_; }
  bool HasVModeRegex() const { return v_mode_regex_.has_value(); }
  bool MatchesVMode(const std::string& preedit) const {
    return v_mode_regex_ && std::regex_search(preedit, *v_mode_regex_);
  }
  void SetVModeRegex(const std::string& pattern) {
    v_mode_regex_.emplace(pattern, std::regex::ECMAScript);
  }
  void SetupClientContext(grpc::ClientContext* context);

  uintptr_t OpenSession();
  void DestroySession(uintptr_t session_id);
  bool ProcessKey(uintptr_t session_id, int keycode, int mask);
  bool GetContext(uintptr_t session_id, service::v2::RimeContextProto* out_context);
  bool GetCommit(uintptr_t session_id, std::string* out_commit);
  bool SelectCandidateOnCurrentPage(uintptr_t session_id, int index);
  bool SelectCandidate(uintptr_t session_id, int index);

private:
  std::unique_ptr<service::v2::RimeService::Stub> stub_;
  
  std::mutex mutex_;
  std::unordered_map<uintptr_t, std::string> sessions_;
  uintptr_t next_id_ = 1;

  int timeout_ms_ = 100;
  bool fallback_on_error_ = true;
  std::optional<std::regex> v_mode_regex_;
};

} // namespace rime

#endif // RIME_GRPC_CLIENT_V2_H_
