#ifndef RIME_REMOTE_SHARED_STATE_H_
#define RIME_REMOTE_SHARED_STATE_H_

#include <memory>
#include <mutex>
#include <string>
#include <unordered_map>
#include <vector>
#include <rime/common.h>
#include "grpc_client.h"

namespace rime {

// Per-candidate data returned from the gRPC backend.
struct RemoteCandidate {
  std::string text;
  std::string comment;
};

// Snapshot of gRPC backend state, shared between Processor/Segmentor/Translator
// within a single Engine instance.
struct RemoteContext {
  // Composition / preedit from gRPC backend
  std::string preedit;
  int cursor_pos = 0;
  int sel_start = 0;
  int sel_end = 0;

  // Menu from gRPC backend
  std::vector<RemoteCandidate> candidates;
  int page_size = 5;
  int page_no = 0;
  bool is_last_page = false;
  int highlighted_candidate_index = 0;
  std::string select_keys;

  std::string commit_text_preview;
};

// Per-engine shared state.  The Processor creates and owns one; Segmentor and
// Translator obtain it through a global registry keyed by Engine*.
class RemoteSharedState {
 public:
  // Whether there is fresh gRPC context waiting to be rendered.
  bool has_context() const { return has_context_; }
  const RemoteContext& context() const { return context_; }

  void SetContext(RemoteContext ctx) {
    context_ = std::move(ctx);
    has_context_ = true;
  }

  void ClearContext() {
    context_ = {};
    has_context_ = false;
  }

  // gRPC client associated with this engine's schema.
  std::shared_ptr<GrpcImeClientV2> client;
  std::string backend_address;
  uintptr_t session_id = 0;
  bool grpc_session_open = false;

  // ASCII mode switch style bindings (keycode -> style).
  enum AsciiSwitchStyle {
    kSwitchNoop = 0,
    kSwitchInlineAscii,
    kSwitchCommitText,
    kSwitchCommitCode,
    kSwitchClear,
  };
  std::unordered_map<int, AsciiSwitchStyle> switch_key_bindings;

 private:
  RemoteContext context_;
  bool has_context_ = false;
};

// Global registry: Engine* -> shared state.
// Thread-safe; components register/unregister on construction/destruction.
class RemoteStateRegistry {
 public:
  static RemoteStateRegistry& instance() {
    static RemoteStateRegistry reg;
    return reg;
  }

  void Register(Engine* engine, an<RemoteSharedState> state) {
    std::lock_guard<std::mutex> lock(mu_);
    states_[engine] = std::move(state);
  }

  void Unregister(Engine* engine) {
    std::lock_guard<std::mutex> lock(mu_);
    states_.erase(engine);
  }

  an<RemoteSharedState> Get(Engine* engine) {
    std::lock_guard<std::mutex> lock(mu_);
    auto it = states_.find(engine);
    return (it != states_.end()) ? it->second : nullptr;
  }

 private:
  RemoteStateRegistry() = default;
  std::mutex mu_;
  std::unordered_map<Engine*, an<RemoteSharedState>> states_;
};

}  // namespace rime

#endif  // RIME_REMOTE_SHARED_STATE_H_
