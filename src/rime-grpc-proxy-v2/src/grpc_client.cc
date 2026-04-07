#include "grpc_client.h"
#include <grpcpp/grpcpp.h>
#include "rime_service.grpc.pb.h"
#include <rime/service.h>
#include <rime/config.h>

namespace rime {

using namespace rime::service::v2;
using grpc::ClientContext;
using grpc::Status;

static std::mutex& GetClientMutex() {
    static std::mutex s_mutex;
    return s_mutex;
}

static std::shared_ptr<GrpcImeClientV2>& GetClientShared() {
    static std::shared_ptr<GrpcImeClientV2> s_client;
    return s_client;
}

std::shared_ptr<GrpcImeClientV2> GrpcImeClientV2::Instance() {
    std::lock_guard<std::mutex> lock(GetClientMutex());
    auto& g_client = GetClientShared();
    if (!g_client) {
        std::string target_address = "127.0.0.1:50051";
        int timeout_ms = 200;
        bool fallback = true;
        std::string v_mode_regex;

        // Use Rime's native Config system to read from user yaml files.
        auto user_dir = rime::Service::instance().deployer().user_data_dir;
        
        rime::Config proxy_config;
        if (proxy_config.LoadFromFile(user_dir / "grpc_proxy.schema.yaml")) {
            proxy_config.GetString("grpc_proxy/backend_address", &target_address);
            proxy_config.GetInt("grpc_proxy/rpc_timeout_ms", &timeout_ms);
            proxy_config.GetBool("grpc_proxy/fallback_on_error", &fallback);
            proxy_config.GetString("grpc_proxy/v_mode_preedit_regex", &v_mode_regex);
        }

        rime::Config custom_config;
        if (custom_config.LoadFromFile(user_dir / "default.custom.yaml")) {
            custom_config.GetString("patch/grpc_proxy/backend_address", &target_address);
            custom_config.GetInt("patch/grpc_proxy/rpc_timeout_ms", &timeout_ms);
            custom_config.GetBool("patch/grpc_proxy/fallback_on_error", &fallback);
            custom_config.GetString("patch/grpc_proxy/v_mode_preedit_regex", &v_mode_regex);
        }
        g_client = std::make_shared<GrpcImeClientV2>(target_address, timeout_ms, fallback);
        if (!v_mode_regex.empty()) {
            g_client->SetVModeRegex(v_mode_regex);
        }
    }
    return g_client;
}

std::shared_ptr<GrpcImeClientV2> GrpcImeClientV2::GetOrCreate(const std::string& target_address, int timeout_ms, bool fallback_on_error) {
    std::lock_guard<std::mutex> lock(GetClientMutex());
    auto& g_client = GetClientShared();
    if (!g_client) {
        g_client = std::make_shared<GrpcImeClientV2>(target_address, timeout_ms, fallback_on_error);
    }
    return g_client;
}

GrpcImeClientV2::GrpcImeClientV2(const std::string& target_address, int timeout_ms, bool fallback_on_error)
    : target_address_(target_address), timeout_ms_(timeout_ms), fallback_on_error_(fallback_on_error) {
  auto channel = grpc::CreateChannel(target_address, grpc::InsecureChannelCredentials());
  stub_ = RimeService::NewStub(channel);
}

void GrpcImeClientV2::SetupClientContext(grpc::ClientContext* context) {
    if (timeout_ms_ > 0) {
        context->set_deadline(std::chrono::system_clock::now() + std::chrono::milliseconds(timeout_ms_));
    }
}
bool GrpcImeClientV2::HasSession(uintptr_t session_id) {
  std::lock_guard<std::mutex> lock(mutex_);
  return sessions_.find(session_id) != sessions_.end();
}

std::string GrpcImeClientV2::FindSession(uintptr_t session_id) {
  std::lock_guard<std::mutex> lock(mutex_);
  auto it = sessions_.find(session_id);
  return (it != sessions_.end()) ? it->second : std::string();
}

GrpcImeClientV2::~GrpcImeClientV2() {
  std::lock_guard<std::mutex> lock(mutex_);
  for (const auto& pair : sessions_) {
    ClientContext context;
    SetupClientContext(&context);
    DestroySessionRequest req;
    req.set_session_id(pair.second);
    DestroySessionResponse resp;
    stub_->DestroySession(&context, req, &resp);
  }
  sessions_.clear();
}

uintptr_t GrpcImeClientV2::OpenSession() {
  ClientContext context;
  SetupClientContext(&context);
  OpenSessionRequest req;
  req.set_schema_id("luna_pinyin");
  OpenSessionResponse resp;
  
  Status status = stub_->OpenSession(&context, req, &resp);
  if (status.ok() && !resp.session_id().empty()) {
    std::lock_guard<std::mutex> lock(mutex_);
    uintptr_t id = next_id_++;
    sessions_[id] = resp.session_id();
    return id;
  }
  return 0;
}

void GrpcImeClientV2::DestroySession(uintptr_t session_id) {
  std::string my_session;
  {
    std::lock_guard<std::mutex> lock(mutex_);
    auto it = sessions_.find(session_id);
    if (it != sessions_.end()) {
      my_session = it->second;
      sessions_.erase(it);
    }
  }

  if (!my_session.empty()) {
    ClientContext context;
    SetupClientContext(&context);
    DestroySessionRequest req;
    req.set_session_id(my_session);
    DestroySessionResponse resp;
    stub_->DestroySession(&context, req, &resp);
  }
}

bool GrpcImeClientV2::ProcessKey(uintptr_t session_id, int keycode, int mask) {
  std::string my_session = FindSession(session_id);
  if (my_session.empty()) return false;

  ClientContext context;
  SetupClientContext(&context);
  ProcessKeyRequest req;
  req.set_session_id(my_session);
  
  auto* ke = req.mutable_key_event();
  ke->set_keycode(keycode);
  ke->set_modifier(mask);

  ProcessKeyResponse resp;
  Status status = stub_->ProcessKey(&context, req, &resp);
  
  if (status.ok()) {
    return resp.accepted();
  }
  return fallback_on_error_ ? false : true;
}

bool GrpcImeClientV2::GetContext(uintptr_t session_id, RimeContextProto* out_context) {
  std::string my_session = FindSession(session_id);
  if (my_session.empty() || !out_context) return false;

  ClientContext context;
  SetupClientContext(&context);
  GetContextRequest req;
  req.set_session_id(my_session);

  GetContextResponse resp;
  Status status = stub_->GetContext(&context, req, &resp);
  
  if (status.ok() && resp.has_context()) {
    *out_context = resp.context();
    return true;
  }
  return false;
}

bool GrpcImeClientV2::GetCommit(uintptr_t session_id, std::string* out_commit) {
  std::string my_session = FindSession(session_id);
  if (my_session.empty() || !out_commit) return false;

  ClientContext context;
  SetupClientContext(&context);
  GetCommitRequest req;
  req.set_session_id(my_session);

  GetCommitResponse resp;
  Status status = stub_->GetCommit(&context, req, &resp);
  
  if (status.ok() && resp.has_commit()) {
    *out_commit = resp.commit_text();
    return true;
  }
  return false;
}

bool GrpcImeClientV2::SelectCandidateOnCurrentPage(uintptr_t session_id, int index) {
  std::string my_session = FindSession(session_id);
  if (my_session.empty()) return false;

  grpc::ClientContext context;
  SetupClientContext(&context);
  service::v2::SelectCandidateRequest req;
  req.set_session_id(my_session);
  req.set_index(index);

  service::v2::SelectCandidateResponse resp;
  grpc::Status status = stub_->SelectCandidateOnCurrentPage(&context, req, &resp);
  
  if (status.ok()) {
    return resp.success();
  }
  return false;
}

bool GrpcImeClientV2::SelectCandidate(uintptr_t session_id, int index) {
  return SelectCandidateOnCurrentPage(session_id, index);
}

} // namespace rime
