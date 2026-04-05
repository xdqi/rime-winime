#ifndef RIME_GRPC_CLIENT_V2_H_
#define RIME_GRPC_CLIENT_V2_H_

#include <string>
#include <memory>
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
  static std::shared_ptr<GrpcImeClientV2> GetOrCreate(Engine* engine, const std::string& target_address);
  
  GrpcImeClientV2(const std::string& target_address);
  ~GrpcImeClientV2();

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
};

} // namespace rime

#endif // RIME_GRPC_CLIENT_V2_H_
