#ifndef RIME_GRPC_CLIENT_V2_H_
#define RIME_GRPC_CLIENT_V2_H_

#include <memory>
#include <mutex>
#include <optional>
#include <regex>
#include <string>
#include <unordered_map>
#include <grpcpp/grpcpp.h>
#include "rime_service.grpc.pb.h"

namespace rime {

class GrpcImeClientV2 {
public:
  // --- Per-address instance pool ---
  // Returns existing client for `target_address`, or creates a new one.
  static std::shared_ptr<GrpcImeClientV2> GetOrCreate(
      const std::string& target_address, int timeout_ms);
  // Look up an existing client by address.  Returns nullptr if none.
  static std::shared_ptr<GrpcImeClientV2> Find(const std::string& target_address);
  // Shut down ALL pooled clients and clear the pool.
  static void ShutdownAll();

  GrpcImeClientV2(const std::string& target_address, int timeout_ms);
  ~GrpcImeClientV2();

  bool HasVModeRegex() const { return v_mode_regex_.has_value(); }
  const std::string& TargetAddress() const { return target_address_; }
  bool MatchesVMode(const std::string& preedit) const {
    return v_mode_regex_ && std::regex_search(preedit, *v_mode_regex_);
  }
  void SetVModeRegex(const std::string& pattern) {
    v_mode_regex_.emplace(pattern, std::regex::ECMAScript);
  }
  void SetupClientContext(grpc::ClientContext* context);

  // Gracefully shut down: destroy remote sessions and release the gRPC
  // channel.  Must be called while gRPC background threads are still alive
  // (i.e. NOT during DllMain/DLL_PROCESS_DETACH).
  void Shutdown();

  // --- Session management (keyed by local rime session_id) ---
  bool HasSession(uintptr_t session_id);
  // Open a gRPC session paired with the given local rime session_id.
  bool OpenSession(uintptr_t session_id,
                   const std::string& schema_id = "luna_pinyin");
  void DestroySession(uintptr_t session_id);
  bool ProcessKey(uintptr_t session_id, int keycode, int mask);
  bool GetContext(uintptr_t session_id, service::v2::RimeContextProto* out_context);
  bool GetCommit(uintptr_t session_id, std::string* out_commit);
  bool SelectCandidateOnCurrentPage(uintptr_t session_id, int index);
  bool SelectCandidate(uintptr_t session_id, int index);

private:
  std::string FindSession(uintptr_t session_id);

  std::unique_ptr<service::v2::RimeService::Stub> stub_;
  
  std::mutex mutex_;
  // Maps local rime session_id -> remote gRPC session string
  std::unordered_map<uintptr_t, std::string> sessions_;

  std::string target_address_;
  int timeout_ms_ = 100;
  std::optional<std::regex> v_mode_regex_;
};

} // namespace rime

#endif // RIME_GRPC_CLIENT_V2_H_
