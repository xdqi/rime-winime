#include <rime_api.h>
#include <rime/component.h>
#include <rime/key_event.h>
#include <rime/registry.h>
#include <rime/schema.h>
#include <cstring>
#include <glog/logging.h>

#include "grpc_key_event_processor.h"
#include "grpc_client.h"

using namespace rime;

// Allocate and copy a C string (caller owns the result).
static char* StrDup(const char* s) {
    if (!s) return nullptr;
    auto* p = new char[std::strlen(s) + 1];
    std::strcpy(p, s);
    return p;
}

// --- Original rime_api function pointers (only the ones we hook) ---
static Bool (*original_process_key)(RimeSessionId, int, int);
static Bool (*original_simulate_key_sequence)(RimeSessionId, const char*);
static Bool (*original_get_context)(RimeSessionId, RIME_FLAVORED(RimeContext)*);
static Bool (*original_get_status)(RimeSessionId, RIME_FLAVORED(RimeStatus)*);
static Bool (*original_get_commit)(RimeSessionId, RIME_FLAVORED(RimeCommit)*);
static Bool (*original_select_candidate)(RimeSessionId, size_t);
static Bool (*original_select_candidate_on_current_page)(RimeSessionId, size_t);

// --------------------------------------------------------------------------
// Schema -> gRPC backend mapping
// --------------------------------------------------------------------------
// We read each grpc_proxy-based schema's config to learn its backend_address.
// This lets us map schema_id -> address -> GrpcImeClientV2 at runtime.
struct GrpcSchemaInfo {
    std::string backend_address;
    int timeout_ms;
    std::string v_mode_preedit_regex;  // optional, applied on first use
};
static std::unordered_map<std::string, GrpcSchemaInfo> g_grpc_schemas;

// --------------------------------------------------------------------------
// Per-session state
// --------------------------------------------------------------------------
struct SessionState {
    bool is_grpc = false;         // current schema is a gRPC one
    std::string schema_id;        // current schema_id
    std::string backend_address;  // current gRPC backend address (if is_grpc)

    // Per-keystroke dedup state (to avoid redundant RPCs)
    bool last_was_keyup_skip = false;
    bool last_process_key_accepted = false;

    // Snapshot of latest context for keyup-skip optimization
    RIME_FLAVORED(RimeContext) last_context;
    bool has_last_context = false;
    Bool last_is_composing = False;
};
static std::unordered_map<uintptr_t, SessionState> g_sessions;

// --------------------------------------------------------------------------
// Helpers — detect schema and resolve client
// --------------------------------------------------------------------------

// Query the REAL rime engine for this session's current schema_id.
static std::string GetLocalSchemaId(RimeSessionId session_id) {
    RimeApi* api = const_cast<RimeApi*>(rime_get_api());
    char buf[256] = {};
    if (api && api->get_current_schema(session_id, buf, sizeof(buf))) {
        return std::string(buf);
    }
    return {};
}

// Get or refresh the SessionState for a rime session.
// Detects schema switches and lazily creates / destroys gRPC sessions.
static SessionState& EnsureSession(RimeSessionId session_id) {
    auto& ss = g_sessions[session_id];

    std::string cur_schema = GetLocalSchemaId(session_id);
    if (cur_schema == ss.schema_id) {
        return ss;  // no change
    }

    // Schema changed (or first call). Tear down old gRPC session if any.
    if (ss.is_grpc && !ss.backend_address.empty()) {
        auto old_client = GrpcImeClientV2::Find(ss.backend_address);
        if (old_client) {
            old_client->DestroySession(session_id);
        }
    }

    ss.schema_id = cur_schema;
    ss.is_grpc = false;
    ss.backend_address.clear();
    ss.last_was_keyup_skip = false;
    ss.last_process_key_accepted = false;
    // Free old context snapshot
    if (ss.has_last_context) {
        delete[] ss.last_context.composition.preedit;
        delete[] ss.last_context.menu.select_keys;
        for (int i = 0; i < ss.last_context.menu.num_candidates; ++i) {
            delete[] ss.last_context.menu.candidates[i].text;
            delete[] ss.last_context.menu.candidates[i].comment;
        }
        delete[] ss.last_context.menu.candidates;
        delete[] ss.last_context.commit_text_preview;
        RIME_STRUCT_CLEAR(ss.last_context);
        ss.has_last_context = false;
    }
    ss.last_is_composing = False;

    // Is the new schema a gRPC schema?
    auto it = g_grpc_schemas.find(cur_schema);
    if (it != g_grpc_schemas.end()) {
        // Lazily create the gRPC client on first use of this backend.
        auto client = GrpcImeClientV2::GetOrCreate(
            it->second.backend_address, it->second.timeout_ms);
        if (client) {
            // Apply v_mode regex config (idempotent).
            if (!it->second.v_mode_preedit_regex.empty() &&
                !client->HasVModeRegex()) {
                client->SetVModeRegex(it->second.v_mode_preedit_regex);
            }
            if (client->OpenSession(session_id)) {
                ss.is_grpc = true;
                ss.backend_address = it->second.backend_address;
                LOG(INFO) << "[grpc_proxy] session " << session_id
                          << " -> gRPC schema '" << cur_schema
                          << "' @ " << ss.backend_address;
            } else {
                LOG(WARNING) << "[grpc_proxy] failed to open gRPC session for schema '"
                             << cur_schema << "'";
            }
        } else {
            LOG(WARNING) << "[grpc_proxy] failed to create client for "
                         << it->second.backend_address;
        }
    } else {
        LOG(INFO) << "[grpc_proxy] session " << session_id
                  << " -> local schema '" << cur_schema << "'";
    }
    return ss;
}

// --------------------------------------------------------------------------
// Context snapshot helpers (per-session)
// --------------------------------------------------------------------------
static void SaveContextSnapshot(SessionState& ss, const RIME_FLAVORED(RimeContext)* src) {
    if (ss.has_last_context) {
        delete[] ss.last_context.composition.preedit;
        delete[] ss.last_context.menu.select_keys;
        for (int i = 0; i < ss.last_context.menu.num_candidates; ++i) {
            delete[] ss.last_context.menu.candidates[i].text;
            delete[] ss.last_context.menu.candidates[i].comment;
        }
        delete[] ss.last_context.menu.candidates;
        delete[] ss.last_context.commit_text_preview;
    }
    RIME_STRUCT_CLEAR(ss.last_context);
    ss.last_context.data_size = src->data_size;

    ss.last_context.composition.length = src->composition.length;
    ss.last_context.composition.cursor_pos = src->composition.cursor_pos;
    ss.last_context.composition.sel_start = src->composition.sel_start;
    ss.last_context.composition.sel_end = src->composition.sel_end;
    ss.last_context.composition.preedit = StrDup(src->composition.preedit);

    ss.last_context.menu.page_size = src->menu.page_size;
    ss.last_context.menu.page_no = src->menu.page_no;
    ss.last_context.menu.is_last_page = src->menu.is_last_page;
    ss.last_context.menu.highlighted_candidate_index = src->menu.highlighted_candidate_index;
    ss.last_context.menu.num_candidates = src->menu.num_candidates;
    ss.last_context.menu.select_keys = StrDup(src->menu.select_keys);
    if (src->menu.num_candidates > 0 && src->menu.candidates) {
        ss.last_context.menu.candidates = new RimeCandidate[src->menu.num_candidates];
        for (int i = 0; i < src->menu.num_candidates; ++i) {
            ss.last_context.menu.candidates[i].text =
                StrDup(src->menu.candidates[i].text ? src->menu.candidates[i].text : "");
            ss.last_context.menu.candidates[i].comment =
                StrDup(src->menu.candidates[i].comment ? src->menu.candidates[i].comment : "");
        }
    }
    if (RIME_STRUCT_HAS_MEMBER(*src, src->commit_text_preview)) {
        ss.last_context.commit_text_preview = StrDup(src->commit_text_preview);
    }
    ss.has_last_context = true;
    ss.last_is_composing = (src->composition.length > 0) ? True : False;
}

static Bool RestoreContextSnapshot(const SessionState& ss,
                                   RIME_FLAVORED(RimeContext)* dst) {
    if (!ss.has_last_context) return False;
    RIME_STRUCT_CLEAR(*dst);
    dst->composition.length = ss.last_context.composition.length;
    dst->composition.cursor_pos = ss.last_context.composition.cursor_pos;
    dst->composition.sel_start = ss.last_context.composition.sel_start;
    dst->composition.sel_end = ss.last_context.composition.sel_end;
    dst->composition.preedit = StrDup(ss.last_context.composition.preedit);
    dst->menu.page_size = ss.last_context.menu.page_size;
    dst->menu.page_no = ss.last_context.menu.page_no;
    dst->menu.is_last_page = ss.last_context.menu.is_last_page;
    dst->menu.highlighted_candidate_index = ss.last_context.menu.highlighted_candidate_index;
    dst->menu.num_candidates = ss.last_context.menu.num_candidates;
    if (ss.last_context.menu.select_keys &&
        RIME_STRUCT_HAS_MEMBER(*dst, dst->menu.select_keys)) {
        dst->menu.select_keys = StrDup(ss.last_context.menu.select_keys);
    }
    if (ss.last_context.menu.num_candidates > 0 && ss.last_context.menu.candidates) {
        dst->menu.candidates = new RimeCandidate[ss.last_context.menu.num_candidates];
        for (int i = 0; i < ss.last_context.menu.num_candidates; ++i) {
            dst->menu.candidates[i].text = StrDup(ss.last_context.menu.candidates[i].text);
            dst->menu.candidates[i].comment = StrDup(ss.last_context.menu.candidates[i].comment);
        }
    }
    if (RIME_STRUCT_HAS_MEMBER(*dst, dst->commit_text_preview)) {
        dst->commit_text_preview = StrDup(ss.last_context.commit_text_preview);
    }
    return True;
}

// --------------------------------------------------------------------------
// Hooked API functions
// --------------------------------------------------------------------------

static Bool MyProcessKey(RimeSessionId session_id, int keycode, int mask) {
    // Always let the local rime engine process first.
    // This handles: switcher (F4, Ctrl+`), ascii_composer (Shift),
    // key_binder, and any other local processors.
    Bool local_handled = original_process_key
        ? original_process_key(session_id, keycode, mask)
        : False;

    auto& ss = EnsureSession(session_id);
    if (!ss.is_grpc) {
        return local_handled;  // not a gRPC schema — done
    }

    // If local rime consumed the key (e.g. Shift toggle, F4 switcher),
    // don't send to gRPC.
    if (local_handled) {
        ss.last_was_keyup_skip = false;
        ss.last_process_key_accepted = false;
        return True;
    }

    // Skip key release events — saves an RPC round-trip.
    if (mask & kReleaseMask) {
        ss.last_was_keyup_skip = true;
        return False;
    }
    ss.last_was_keyup_skip = false;

    // If local rime's ascii_mode is ON, don't forward to gRPC.
    // Let keys pass through as English characters.
    {
        RimeApi* api = const_cast<RimeApi*>(rime_get_api());
        if (api && api->get_option &&
            api->get_option(session_id, "ascii_mode")) {
            ss.last_process_key_accepted = false;
            return False;
        }
    }

    // Forward to gRPC backend.
    auto client = GrpcImeClientV2::Find(ss.backend_address);
    if (client && client->HasSession(session_id)) {
        bool res = client->ProcessKey(session_id, keycode, mask);
        ss.last_process_key_accepted = res;
        LOG(INFO) << "[grpc_proxy] ProcessKey(session=" << session_id
                  << ", key=" << keycode << ", mask=" << mask
                  << ") -> " << res;
        return res ? True : False;
    }

    ss.last_process_key_accepted = false;
    return False;
}

static Bool MySimulateKeySequence(RimeSessionId session_id,
                                  const char* key_sequence) {
    auto& ss = EnsureSession(session_id);
    if (!ss.is_grpc) {
        return original_simulate_key_sequence
            ? original_simulate_key_sequence(session_id, key_sequence) : False;
    }
    rime::KeySequence keys;
    if (!keys.Parse(key_sequence)) {
        LOG(ERROR) << "[grpc_proxy] error parsing input: '" << key_sequence << "'";
        return False;
    }
    for (const rime::KeyEvent& key : keys) {
        MyProcessKey(session_id, key.keycode(), key.modifier());
    }
    return True;
}

static Bool MyGetContext(RimeSessionId session_id,
                         RIME_FLAVORED(RimeContext)* context) {
    if (!context || context->data_size <= 0) return False;

    auto& ss = EnsureSession(session_id);
    if (!ss.is_grpc) {
        return original_get_context
            ? original_get_context(session_id, context) : False;
    }

    // In ascii_mode, no Chinese composition — return empty context.
    {
        RimeApi* api = const_cast<RimeApi*>(rime_get_api());
        if (api && api->get_option &&
            api->get_option(session_id, "ascii_mode")) {
            RIME_STRUCT_CLEAR(*context);
            return True;
        }
    }

    // After keyup skip, return saved snapshot.
    if (ss.last_was_keyup_skip && ss.has_last_context) {
        return RestoreContextSnapshot(ss, context);
    }

    auto client = GrpcImeClientV2::Find(ss.backend_address);
    if (!client || !client->HasSession(session_id)) {
        return original_get_context
            ? original_get_context(session_id, context) : False;
    }

    LOG(INFO) << "[grpc_proxy] GetContext(session=" << session_id << ")";

    service::v2::RimeContextProto proto;
    if (client->GetContext(session_id, &proto)) {
        RIME_STRUCT_CLEAR(*context);
        
        if (proto.has_composition()) {
            const auto& comp = proto.composition();
            context->composition.length = comp.length();
            context->composition.cursor_pos = comp.cursor_pos();
            context->composition.sel_start = comp.sel_start();
            context->composition.sel_end = comp.sel_end();
            if (!comp.preedit().empty()) {
                context->composition.preedit = StrDup(comp.preedit().c_str());
            }
        }
        
        if (proto.has_menu()) {
            const auto& menu = proto.menu();
            context->menu.page_size = menu.page_size();
            context->menu.page_no = menu.page_no();
            context->menu.is_last_page = menu.is_last_page() ? True : False;
            context->menu.highlighted_candidate_index = menu.highlighted_candidate_index();
            context->menu.num_candidates = menu.num_candidates();
            
            if (!menu.select_keys().empty() &&
                RIME_STRUCT_HAS_MEMBER(*context, context->menu.select_keys)) {
                context->menu.select_keys = StrDup(menu.select_keys().c_str());
            }
            
            if (menu.num_candidates() > 0) {
                context->menu.candidates = new RimeCandidate[menu.num_candidates()];
                for (int i = 0; i < menu.num_candidates(); ++i) {
                    const auto& cand = menu.candidates(i);
                    context->menu.candidates[i].text =
                        StrDup(cand.text().empty() ? "" : cand.text().c_str());
                    context->menu.candidates[i].comment =
                        StrDup(cand.comment().empty() ? "" : cand.comment().c_str());
                }
            }
        }
        
        if (!proto.commit_text_preview().empty() &&
            RIME_STRUCT_HAS_MEMBER(*context, context->commit_text_preview)) {
            context->commit_text_preview =
                StrDup(proto.commit_text_preview().c_str());
        }

        // Override select_keys based on v_mode_preedit_regex config
        if (context->composition.preedit && context->menu.page_size > 0) {
            auto cli = GrpcImeClientV2::Find(ss.backend_address);
            if (cli && cli->HasVModeRegex()) {
                const char* alpha_keys = "abcdefghij";
                const char* num_keys   = "1234567890";
                bool is_vmode = cli->MatchesVMode(context->composition.preedit);
                const char* src_keys = is_vmode ? alpha_keys : num_keys;
                int n = context->menu.page_size;
                if (n > 10) n = 10;
                delete[] context->menu.select_keys;
                context->menu.select_keys = new char[n + 1];
                std::memcpy(context->menu.select_keys, src_keys, n);
                context->menu.select_keys[n] = '\0';
            }
        }
        
        SaveContextSnapshot(ss, context);
        return True;
    }
    return False;
}

static Bool MyGetStatus(RimeSessionId session_id,
                        RIME_FLAVORED(RimeStatus)* status) {
    if (!status || status->data_size <= 0) return False;

    auto& ss = EnsureSession(session_id);
    if (!ss.is_grpc) {
        return original_get_status
            ? original_get_status(session_id, status) : False;
    }

    // After keyup skip, derive from saved snapshot.
    if (ss.last_was_keyup_skip) {
        RIME_STRUCT_CLEAR(*status);
        status->is_composing = ss.last_is_composing;
        // Let local rime report ascii_mode via its own option.
        RimeApi* api = const_cast<RimeApi*>(rime_get_api());
        if (api && api->get_option) {
            status->is_ascii_mode = api->get_option(session_id, "ascii_mode");
        }
        status->schema_id = StrDup(ss.schema_id.c_str());
        status->schema_name = StrDup("gRPC Proxy");
        return True;
    }

    LOG(INFO) << "[grpc_proxy] GetStatus(session=" << session_id << ")";

    auto client = GrpcImeClientV2::Find(ss.backend_address);
    if (!client || !client->HasSession(session_id)) {
        return original_get_status
            ? original_get_status(session_id, status) : False;
    }

    service::v2::RimeContextProto proto;
    if (client->GetContext(session_id, &proto)) {
        RIME_STRUCT_CLEAR(*status);
        status->is_composing = proto.has_composition();
        RimeApi* api = const_cast<RimeApi*>(rime_get_api());
        if (api && api->get_option) {
            status->is_ascii_mode = api->get_option(session_id, "ascii_mode");
        }
        status->schema_id = StrDup(ss.schema_id.c_str());
        status->schema_name = StrDup("gRPC Proxy");
        ss.last_is_composing = status->is_composing;
        return True;
    }
    return False;
}

static Bool MyGetCommit(RimeSessionId session_id,
                        RIME_FLAVORED(RimeCommit)* commit) {
    if (!commit || commit->data_size <= 0) return False;

    auto& ss = EnsureSession(session_id);
    if (!ss.is_grpc) {
        return original_get_commit
            ? original_get_commit(session_id, commit) : False;
    }

    // After keyup skip or unaccepted key, no new commit possible.
    if (ss.last_was_keyup_skip || !ss.last_process_key_accepted) {
        return False;
    }

    auto client = GrpcImeClientV2::Find(ss.backend_address);
    if (!client || !client->HasSession(session_id)) {
        return original_get_commit
            ? original_get_commit(session_id, commit) : False;
    }

    LOG(INFO) << "[grpc_proxy] GetCommit(session=" << session_id << ")";

    std::string text;
    if (client->GetCommit(session_id, &text) && !text.empty()) {
        RIME_STRUCT_CLEAR(*commit);
        commit->text = StrDup(text.c_str());
        return True;
    }
    return False;
}

static Bool MySelectCandidate(RimeSessionId session_id, size_t index) {
    auto& ss = EnsureSession(session_id);
    if (!ss.is_grpc) {
        return original_select_candidate
            ? original_select_candidate(session_id, index) : False;
    }
    auto client = GrpcImeClientV2::Find(ss.backend_address);
    if (client) {
        return client->SelectCandidate(session_id, static_cast<int>(index))
            ? True : False;
    }
    return False;
}

static Bool MySelectCandidateOnCurrentPage(RimeSessionId session_id,
                                           size_t index) {
    auto& ss = EnsureSession(session_id);
    if (!ss.is_grpc) {
        return original_select_candidate_on_current_page
            ? original_select_candidate_on_current_page(session_id, index)
            : False;
    }
    auto client = GrpcImeClientV2::Find(ss.backend_address);
    if (client) {
        return client->SelectCandidateOnCurrentPage(
            session_id, static_cast<int>(index)) ? True : False;
    }
    return False;
}

// --------------------------------------------------------------------------
// Module init / finalize
// --------------------------------------------------------------------------

static void rime_grpc_proxy_v2_initialize() {
  LOG(INFO) << "[grpc_proxy] rime_grpc_proxy_v2_initialize called!";
  Registry& r = Registry::instance();
  r.Register("grpc_key_event_processor", new Component<GrpcKeyEventProcessor>);

  // Scan all schema files for grpc_proxy/backend_address.
  // This builds the g_grpc_schemas map so we know which schemas are gRPC-based.
  RimeApi* api = const_cast<RimeApi*>(rime_get_api());
  if (api) {
      RimeSchemaList schema_list = {};
      if (api->get_schema_list(&schema_list)) {
          for (size_t i = 0; i < schema_list.size; ++i) {
              const char* sid = schema_list.list[i].schema_id;
              if (!sid) continue;
              RimeConfig cfg = {};
              if (api->schema_open(sid, &cfg)) {
                  char addr_buf[256] = {};
                  if (api->config_get_string(&cfg, "grpc_proxy/backend_address",
                                             addr_buf, sizeof(addr_buf))) {
                      GrpcSchemaInfo info;
                      info.backend_address = addr_buf;
                      info.timeout_ms = 200;
                      int t = 0;
                      if (api->config_get_int(&cfg, "grpc_proxy/rpc_timeout_ms", &t) && t > 0) {
                          info.timeout_ms = t;
                      }
                      // Save v_mode_regex config for lazy application.
                      char regex_buf[256] = {};
                      if (api->config_get_string(&cfg, "grpc_proxy/v_mode_preedit_regex",
                                                 regex_buf, sizeof(regex_buf)) &&
                          regex_buf[0]) {
                          info.v_mode_preedit_regex = regex_buf;
                      }
                      g_grpc_schemas[sid] = info;
                      // NOTE: gRPC client is NOT created here — it will be
                      // lazily created in EnsureSession() when this schema
                      // is first activated, avoiding startup blocking.
                      LOG(INFO) << "[grpc_proxy] registered gRPC schema '"
                                << sid << "' -> " << info.backend_address
                                << " timeout=" << info.timeout_ms << "ms";
                  }
                  api->config_close(&cfg);
              }
          }
          api->free_schema_list(&schema_list);
      }

      // Hook only the API functions we need.
      // Session management (create/destroy/find) is NOT hooked — rime
      // manages local sessions so that switcher, ascii_composer, etc. work.
      original_process_key = api->process_key;
      original_simulate_key_sequence = api->simulate_key_sequence;
      original_get_context = api->get_context;
      original_get_status = api->get_status;
      original_get_commit = api->get_commit;
      original_select_candidate = api->select_candidate;
      original_select_candidate_on_current_page = api->select_candidate_on_current_page;

      api->process_key = MyProcessKey;
      api->simulate_key_sequence = MySimulateKeySequence;
      api->get_context = MyGetContext;
      api->get_status = MyGetStatus;
      api->get_commit = MyGetCommit;
      api->select_candidate = MySelectCandidate;
      api->select_candidate_on_current_page = MySelectCandidateOnCurrentPage;
      LOG(INFO) << "[grpc_proxy] RimeApi hooks installed (process_key, "
                   "get_context, get_status, get_commit, select_candidate).";
  } else {
      LOG(ERROR) << "[grpc_proxy] rime_get_api() returned NULL!";
  }
}

static void rime_grpc_proxy_v2_finalize() {
  GrpcImeClientV2::ShutdownAll();
  g_sessions.clear();
  LOG(INFO) << "[grpc_proxy] finalized, all gRPC clients destroyed.";
}

RIME_REGISTER_MODULE(grpc_proxy_v2)
