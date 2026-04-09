#include "grpc_client.h"

#include <algorithm>
#include <cctype>
#include <chrono>
#include <cstdlib>
#include <map>
#include <mutex>
#include <utility>

#include <grpcpp/grpcpp.h>

#include <rime/common.h>
#include <rime/engine.h>

#if __has_include("ime_proxy.grpc.pb.h")
#include "ime_proxy.grpc.pb.h"
#elif __has_include("../build/generated/ime_proxy.grpc.pb.h")
#include "../build/generated/ime_proxy.grpc.pb.h"
#else
#error "Generated gRPC header ime_proxy.grpc.pb.h not found. Run CMake build first."
#endif

namespace rime {
namespace grpc_proxy {

namespace {

using std::chrono::milliseconds;
namespace ime_proto = ::ime::gateway::v1;

std::mutex g_registry_mutex;
std::map<Engine*, std::weak_ptr<GrpcImeClient>> g_registry;

static int64_t UnixNowMs() {
  const auto now = std::chrono::system_clock::now();
  const auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
      now.time_since_epoch());
  return ms.count();
}

static void FailFastIfNeeded(bool enabled, const std::string& reason) {
  if (!enabled) {
    return;
  }
  LOG(ERROR) << "grpc_proxy debug_stop_mode triggered: " << reason;
  std::abort();
}

static bool PerKeyTraceEnabled() {
  static const bool enabled = []() {
    const char* raw = std::getenv("IME_GRPC_TRACE_PER_KEY");
    if (!raw) {
      return false;
    }

    std::string value(raw);
    std::transform(value.begin(), value.end(), value.begin(),
                   [](unsigned char c) {
                     return static_cast<char>(std::tolower(c));
                   });
    return value == "1" || value == "true" || value == "yes" ||
           value == "on";
  }();

  return enabled;
}

static std::string CandidatePreview(const QueryResult& result, size_t limit) {
  if (result.candidates.empty() || limit == 0) {
    return "<none>";
  }

  const size_t count = std::min(result.candidates.size(), limit);
  std::string out;
  for (size_t i = 0; i < count; ++i) {
    if (i > 0) {
      out.append(" | ");
    }
    out.append(result.candidates[i].text);
  }
  if (result.candidates.size() > count) {
    out.append(" | ...");
  }
  return out;
}

static std::string KeyEventLabel(const KeyEvent& key_event) {
  const uint32_t code = static_cast<uint32_t>(key_event.keycode());
  const uint32_t vk = code & 0xff;

  std::string out = "keycode=" + std::to_string(code) +
                    " vk=" + std::to_string(vk);
  if (vk >= 0x20 && vk < 0x7f) {
    out.append(" ch='");
    out.push_back(static_cast<char>(vk));
    out.push_back('\'');
  }

  if (key_event.shift() || key_event.ctrl() || key_event.alt()) {
    out.append(" mods=");
    out.append(key_event.shift() ? "S" : "-");
    out.append(key_event.ctrl() ? "C" : "-");
    out.append(key_event.alt() ? "A" : "-");
  }

  out.append(key_event.release() ? " up" : " down");
  return out;
}

}  // namespace

struct GrpcImeClient::Impl {
  explicit Impl(const GrpcClientConfig& in_cfg)
      : cfg(in_cfg),
        channel(grpc::CreateChannel(
            cfg.host + ":" + std::to_string(cfg.port),
            grpc::InsecureChannelCredentials())),
      stub(ime_proto::ImeGateway::NewStub(channel)) {}

  GrpcClientConfig cfg;
  std::mutex mu;
  std::shared_ptr<grpc::Channel> channel;
  std::unique_ptr<ime_proto::ImeGateway::Stub> stub;
  std::string session_id;
  uint64_t seq = 0;
  QueryResult cached_query;
  bool has_cached_query = false;
};

std::shared_ptr<GrpcImeClient> GrpcImeClient::GetOrCreate(
    Engine* engine,
    const GrpcClientConfig& cfg) {
  if (!engine) {
    return std::make_shared<GrpcImeClient>(cfg);
  }

  std::lock_guard<std::mutex> lock(g_registry_mutex);
  auto it = g_registry.find(engine);
  if (it != g_registry.end()) {
    if (auto shared = it->second.lock()) {
      return shared;
    }
  }

  auto created = std::make_shared<GrpcImeClient>(cfg);
  g_registry[engine] = created;
  return created;
}

GrpcImeClient::GrpcImeClient(const GrpcClientConfig& cfg)
    : impl_(new Impl(cfg)) {}

GrpcImeClient::~GrpcImeClient() = default;

bool GrpcImeClient::EnsureSessionLocked() {
  if (!impl_->session_id.empty()) {
    return true;
  }

  ime_proto::OpenSessionRequest req;
  req.set_frontend_id(impl_->cfg.frontend_id);
  req.set_schema_id(impl_->cfg.schema_id);
  req.set_want_prewarmed_worker(true);

  ime_proto::OpenSessionResponse resp;
  grpc::ClientContext ctx;
  ctx.set_deadline(std::chrono::system_clock::now() +
                   milliseconds(impl_->cfg.timeout_ms));

  const grpc::Status status = impl_->stub->OpenSession(&ctx, req, &resp);
  if (!status.ok()) {
    LOG(ERROR) << "grpc_proxy OpenSession transport failed: code="
               << status.error_code() << " message=" << status.error_message();
    FailFastIfNeeded(impl_->cfg.debug_stop_mode,
                     "OpenSession transport failure");
    return false;
  }

  if (resp.session_id().empty()) {
    LOG(ERROR) << "grpc_proxy OpenSession returned empty session_id";
    FailFastIfNeeded(impl_->cfg.debug_stop_mode,
                     "OpenSession returned empty session_id");
    return false;
  }

  impl_->session_id = resp.session_id();
  impl_->seq = 0;
  impl_->has_cached_query = false;

  DLOG(INFO) << "grpc_proxy session opened: " << impl_->session_id
             << " worker=" << resp.worker_id();
  return true;
}

bool GrpcImeClient::HandleStatusLocked(const char* callsite,
                                       const std::string& session_id,
                                       const std::string& error_code,
                                       const std::string& error_message,
                                       bool ok_transport) {
  if (!ok_transport) {
    FailFastIfNeeded(impl_->cfg.debug_stop_mode,
                     std::string(callsite) + " transport failure");
    return false;
  }

  if (!error_code.empty()) {
    LOG(ERROR) << "grpc_proxy " << callsite << " failed: session_id="
               << session_id << " error_code=" << error_code
               << " error_message=" << error_message;
    FailFastIfNeeded(impl_->cfg.debug_stop_mode,
                     std::string(callsite) + " backend error");
    return false;
  }

  return true;
}

bool GrpcImeClient::SendKeyEvent(const KeyEvent& key_event) {
  std::lock_guard<std::mutex> lock(impl_->mu);

  if (!EnsureSessionLocked()) {
    return false;
  }

  ime_proto::SendKeyEventRequest req;
  req.set_session_id(impl_->session_id);

  auto* event = req.mutable_key_event();
  const uint64_t seq = ++impl_->seq;
  event->set_seq(seq);
  event->set_key_down(!key_event.release());
  event->set_virtual_key(static_cast<uint32_t>(key_event.keycode() & 0xff));
  event->set_scan_code(0);
  event->set_shift(key_event.shift());
  event->set_ctrl(key_event.ctrl());
  event->set_alt(key_event.alt());
  event->set_repeated(false);
  event->set_extended(false);
  event->set_timestamp_ms(UnixNowMs());
  event->set_source_keycode(static_cast<uint32_t>(key_event.keycode()));
  event->set_source_modifier(static_cast<uint32_t>(key_event.modifier()));

  ime_proto::SendKeyEventResponse resp;
  grpc::ClientContext ctx;
  ctx.set_deadline(std::chrono::system_clock::now() +
                   milliseconds(impl_->cfg.timeout_ms));

  const grpc::Status status = impl_->stub->SendKeyEvent(&ctx, req, &resp);
  const bool ok = HandleStatusLocked("SendKeyEvent", impl_->session_id,
                                     resp.error_code(), resp.error_message(),
                                     status.ok());
  if (!ok) {
    return false;
  }

  if (!key_event.release()) {
    QueryResult prefetched;
    if (!QueryCandidatesLocked("", impl_->cfg.max_candidates, &prefetched,
                               false)) {
      if (PerKeyTraceEnabled()) {
        LOG(WARNING) << "grpc_proxy per-key prefetch failed: "
                     << KeyEventLabel(key_event);
      } else {
        DLOG(INFO) << "grpc_proxy per-key QueryCandidates prefetch failed";
      }
    } else if (PerKeyTraceEnabled()) {
      LOG(INFO) << "grpc_proxy per-key snapshot: " << KeyEventLabel(key_event)
                << " comp='" << prefetched.composition << "'"
                << " read='" << prefetched.reading << "'"
                << " cand_n=" << prefetched.candidates.size()
                << " top=[" << CandidatePreview(prefetched, 5) << "]";
    }
  }

  return true;
}

bool GrpcImeClient::QueryCandidates(const std::string& input,
                                    int max_candidates,
                                    QueryResult* out) {
  if (!out) {
    return false;
  }

  std::lock_guard<std::mutex> lock(impl_->mu);

  if (!EnsureSessionLocked()) {
    return false;
  }

  return QueryCandidatesLocked(input, max_candidates, out, true);
}

bool GrpcImeClient::QueryCandidatesLocked(const std::string& input,
                                          int max_candidates,
                                          QueryResult* out,
                                          bool allow_cached) {
  if (!out) {
    return false;
  }

  const int clamped_max = std::max(1, max_candidates);
  if (allow_cached && impl_->has_cached_query && !input.empty()) {
    const bool composition_match = impl_->cached_query.composition == input;
    const bool reading_match = impl_->cached_query.reading == input;
    if (composition_match || reading_match) {
      *out = impl_->cached_query;
      if (static_cast<int>(out->candidates.size()) > clamped_max) {
        out->candidates.resize(static_cast<size_t>(clamped_max));
      }
      out->page_size = clamped_max;
      if (PerKeyTraceEnabled()) {
        LOG(INFO) << "grpc_proxy query cache hit: input='" << input
                  << "' comp='" << out->composition << "'"
                  << " read='" << out->reading << "'"
                  << " cand_n=" << out->candidates.size()
                  << " top=[" << CandidatePreview(*out, 5) << "]";
      }
      return true;
    }
  }

  ime_proto::QueryCandidatesRequest req;
  req.set_session_id(impl_->session_id);
  req.set_seq(++impl_->seq);
  req.set_input_snapshot(input);
  req.set_max_candidates(static_cast<uint32_t>(clamped_max));

  ime_proto::QueryCandidatesResponse resp;
  grpc::ClientContext ctx;
  ctx.set_deadline(std::chrono::system_clock::now() +
                   milliseconds(impl_->cfg.timeout_ms));

  const grpc::Status status = impl_->stub->QueryCandidates(&ctx, req, &resp);
  if (!HandleStatusLocked("QueryCandidates", impl_->session_id,
                          resp.error_code(), resp.error_message(),
                          status.ok())) {
    return false;
  }

  out->composition = resp.composition();
  out->reading = resp.reading();
  out->selected_index = static_cast<int>(resp.selected_index());
  out->page_size = static_cast<int>(resp.page_size());
  out->candidates.clear();
  out->candidates.reserve(resp.candidates_size());

  for (const auto& item : resp.candidates()) {
    out->candidates.push_back(
        CandidateView{item.text(), item.comment(), item.quality()});
  }

  impl_->cached_query = *out;
  impl_->has_cached_query = true;

  if (PerKeyTraceEnabled()) {
    LOG(INFO) << "grpc_proxy query rpc: input='" << input
              << "' comp='" << out->composition << "'"
              << " read='" << out->reading << "'"
              << " cand_n=" << out->candidates.size()
              << " top=[" << CandidatePreview(*out, 5) << "]";
  }

  return true;
}

bool GrpcImeClient::CommitSelection(const std::string& committed_text,
                                    int candidate_index,
                                    uint64_t seq_hint) {
  std::lock_guard<std::mutex> lock(impl_->mu);

  if (!EnsureSessionLocked()) {
    return false;
  }

  ime_proto::CommitSelectionRequest req;
  req.set_session_id(impl_->session_id);
  req.set_seq(seq_hint > 0 ? seq_hint : ++impl_->seq);
  req.set_candidate_index(
      candidate_index >= 0 ? static_cast<uint32_t>(candidate_index) : 0U);
  req.set_committed_text(committed_text);

  ime_proto::CommitSelectionResponse resp;
  grpc::ClientContext ctx;
  ctx.set_deadline(std::chrono::system_clock::now() +
                   milliseconds(impl_->cfg.timeout_ms));

  const grpc::Status status = impl_->stub->CommitSelection(&ctx, req, &resp);
  const bool ok = HandleStatusLocked("CommitSelection", impl_->session_id,
                                     resp.error_code(), resp.error_message(),
                                     status.ok());
  if (ok) {
    impl_->has_cached_query = false;
  }
  return ok;
}

}  // namespace grpc_proxy
}  // namespace rime
