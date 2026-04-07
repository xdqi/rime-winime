#include <rime_api.h>
#include <rime/key_event.h>
#include <cstring>
#include <glog/logging.h>

#include "grpc_key_event_processor.h"
#include "grpc_client.h"

using namespace rime;

static RimeSessionId (*original_create_session)();
static Bool (*original_destroy_session)(RimeSessionId);
static Bool (*original_find_session)(RimeSessionId);
static Bool (*original_process_key)(RimeSessionId, int, int);
static Bool (*original_simulate_key_sequence)(RimeSessionId, const char*);
static Bool (*original_get_context)(RimeSessionId, RIME_FLAVORED(RimeContext)*);
static Bool (*original_get_status)(RimeSessionId, RIME_FLAVORED(RimeStatus)*);
static Bool (*original_get_commit)(RimeSessionId, RIME_FLAVORED(RimeCommit)*);
static Bool (*original_select_candidate)(RimeSessionId, size_t);
static Bool (*original_select_candidate_on_current_page)(RimeSessionId, size_t);

// --- Per-keystroke state to eliminate redundant RPCs ---
// Tracks whether the most recent MyProcessKey was locally skipped (keyup)
// or returned false (key not consumed). In either case, subsequent
// GetContext/GetCommit/GetStatus can reuse the previous result instead
// of making a new RPC, because IME state didn't change.
// These are NOT cross-keystroke caches — they are refreshed on every
// keydown ProcessKey that actually goes to the host.
static bool g_last_was_keyup_skip = false;
static bool g_last_process_key_accepted = false;
// Snapshot of the latest context/status obtained from a real RPC.
// Used to serve keyup-triggered GetContext/GetStatus without RPC.
static RIME_FLAVORED(RimeContext) g_last_context;
static bool g_has_last_context = false;
static Bool g_last_is_composing = False;

static RimeSessionId MyCreateSession() {
    auto client = GrpcImeClientV2::Instance();
    if (client) {
        LOG(INFO) << "[grpc_proxy] MyCreateSession called! Connecting to backend at: " << client->TargetAddress();
        auto id = client->OpenSession();
        LOG(INFO) << "[grpc_proxy] MyCreateSession returned " << id;
        
        // Fallback if RPC failed and fallback is enabled
        if (id == 0 && client->FallbackOnError() && original_create_session) {
            LOG(WARNING) << "[grpc_proxy] RPC OpenSession failed. Falling back to local Rime engine.";
            return original_create_session();
        }
        return id;
    }
    LOG(ERROR) << "[grpc_proxy] MyCreateSession failed because no client!";
    return original_create_session ? original_create_session() : 0;
}

static Bool MyDestroySession(RimeSessionId session_id) {
    auto client = GrpcImeClientV2::Instance();
    if (client && client->HasSession(session_id)) {
        client->DestroySession(session_id);
        return True;
    }
    return original_destroy_session ? original_destroy_session(session_id) : False;
}

static Bool MyFindSession(RimeSessionId session_id) {
    auto client = GrpcImeClientV2::Instance();
    if (client && client->HasSession(session_id)) {
        return True;
    }
    return original_find_session ? original_find_session(session_id) : False;
}

static Bool MyProcessKey(RimeSessionId session_id, int keycode, int mask) {
    // Skip key release events — Sogou IME never consumes them (always BOOL(0)),     
    // but each RPC costs ~10ms. Skipping halves the RPC count.
    // kReleaseMask = 1 << 30
    if (mask & (1 << 30)) {
        g_last_was_keyup_skip = true;
        return False;
    }
    g_last_was_keyup_skip = false;

    LOG(INFO) << "[grpc_proxy] MyProcessKey called(session=" << session_id << ", keycode=" << keycode << ", mask=" << mask << ")";
    auto client = GrpcImeClientV2::Instance();
    if (client && client->HasSession(session_id)) {
        bool res = client->ProcessKey(session_id, keycode, mask);
        g_last_process_key_accepted = res;
        LOG(INFO) << "[grpc_proxy] MyProcessKey returning " << res;
        return res;
    }
    
    if (original_process_key) {
        return original_process_key(session_id, keycode, mask);
    }
    g_last_process_key_accepted = false;
    return False;
}

static Bool MySimulateKeySequence(RimeSessionId session_id, const char* key_sequence) {
    LOG(INFO) << "[grpc_proxy] MySimulateKeySequence called: " << key_sequence;
    auto client = GrpcImeClientV2::Instance();
    if (!client || !client->HasSession(session_id)) {
        return original_simulate_key_sequence ? original_simulate_key_sequence(session_id, key_sequence) : False;
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

// Helper: deep-copy a RimeContext into the global snapshot.
// Frees previous snapshot data first.
static void SaveContextSnapshot(const RIME_FLAVORED(RimeContext)* src) {
    // Free old snapshot allocations
    if (g_has_last_context) {
        delete[] g_last_context.composition.preedit;
        delete[] g_last_context.menu.select_keys;
        for (int i = 0; i < g_last_context.menu.num_candidates; ++i) {
            delete[] g_last_context.menu.candidates[i].text;
            delete[] g_last_context.menu.candidates[i].comment;
        }
        delete[] g_last_context.menu.candidates;
        delete[] g_last_context.commit_text_preview;
    }
    RIME_STRUCT_CLEAR(g_last_context);
    g_last_context.data_size = src->data_size;

    g_last_context.composition.length = src->composition.length;
    g_last_context.composition.cursor_pos = src->composition.cursor_pos;
    g_last_context.composition.sel_start = src->composition.sel_start;
    g_last_context.composition.sel_end = src->composition.sel_end;
    if (src->composition.preedit) {
        g_last_context.composition.preedit = new char[strlen(src->composition.preedit) + 1];
        std::strcpy(g_last_context.composition.preedit, src->composition.preedit);
    }

    g_last_context.menu.page_size = src->menu.page_size;
    g_last_context.menu.page_no = src->menu.page_no;
    g_last_context.menu.is_last_page = src->menu.is_last_page;
    g_last_context.menu.highlighted_candidate_index = src->menu.highlighted_candidate_index;
    g_last_context.menu.num_candidates = src->menu.num_candidates;
    if (src->menu.select_keys) {
        g_last_context.menu.select_keys = new char[strlen(src->menu.select_keys) + 1];
        std::strcpy(g_last_context.menu.select_keys, src->menu.select_keys);
    }
    if (src->menu.num_candidates > 0 && src->menu.candidates) {
        g_last_context.menu.candidates = new RimeCandidate[src->menu.num_candidates];
        for (int i = 0; i < src->menu.num_candidates; ++i) {
            auto* t = src->menu.candidates[i].text;
            auto* c = src->menu.candidates[i].comment;
            g_last_context.menu.candidates[i].text = new char[strlen(t ? t : "") + 1];
            std::strcpy(g_last_context.menu.candidates[i].text, t ? t : "");
            g_last_context.menu.candidates[i].comment = new char[strlen(c ? c : "") + 1];
            std::strcpy(g_last_context.menu.candidates[i].comment, c ? c : "");
        }
    }
    if (RIME_STRUCT_HAS_MEMBER(*src, src->commit_text_preview) && src->commit_text_preview) {
        g_last_context.commit_text_preview = new char[strlen(src->commit_text_preview) + 1];
        std::strcpy(g_last_context.commit_text_preview, src->commit_text_preview);
    }
    g_has_last_context = true;
    g_last_is_composing = (src->composition.length > 0) ? True : False;
}

// Helper: copy the saved snapshot into a caller-provided RimeContext.
static Bool RestoreContextSnapshot(RIME_FLAVORED(RimeContext)* dst) {
    if (!g_has_last_context) return False;
    RIME_STRUCT_CLEAR(*dst);
    dst->composition.length = g_last_context.composition.length;
    dst->composition.cursor_pos = g_last_context.composition.cursor_pos;
    dst->composition.sel_start = g_last_context.composition.sel_start;
    dst->composition.sel_end = g_last_context.composition.sel_end;
    if (g_last_context.composition.preedit) {
        dst->composition.preedit = new char[strlen(g_last_context.composition.preedit) + 1];
        std::strcpy(dst->composition.preedit, g_last_context.composition.preedit);
    }
    dst->menu.page_size = g_last_context.menu.page_size;
    dst->menu.page_no = g_last_context.menu.page_no;
    dst->menu.is_last_page = g_last_context.menu.is_last_page;
    dst->menu.highlighted_candidate_index = g_last_context.menu.highlighted_candidate_index;
    dst->menu.num_candidates = g_last_context.menu.num_candidates;
    if (g_last_context.menu.select_keys && RIME_STRUCT_HAS_MEMBER(*dst, dst->menu.select_keys)) {
        dst->menu.select_keys = new char[strlen(g_last_context.menu.select_keys) + 1];
        std::strcpy(dst->menu.select_keys, g_last_context.menu.select_keys);
    }
    if (g_last_context.menu.num_candidates > 0 && g_last_context.menu.candidates) {
        dst->menu.candidates = new RimeCandidate[g_last_context.menu.num_candidates];
        for (int i = 0; i < g_last_context.menu.num_candidates; ++i) {
            auto* t = g_last_context.menu.candidates[i].text;
            auto* c = g_last_context.menu.candidates[i].comment;
            dst->menu.candidates[i].text = new char[strlen(t) + 1];
            std::strcpy(dst->menu.candidates[i].text, t);
            dst->menu.candidates[i].comment = new char[strlen(c) + 1];
            std::strcpy(dst->menu.candidates[i].comment, c);
        }
    }
    if (RIME_STRUCT_HAS_MEMBER(*dst, dst->commit_text_preview) && g_last_context.commit_text_preview) {
        dst->commit_text_preview = new char[strlen(g_last_context.commit_text_preview) + 1];
        std::strcpy(dst->commit_text_preview, g_last_context.commit_text_preview);
    }
    return True;
}

static Bool MyGetContext(RimeSessionId session_id, RIME_FLAVORED(RimeContext)* context) {
    if (!context || context->data_size <= 0) return False;

    auto client = GrpcImeClientV2::Instance();
    if (!client || !client->HasSession(session_id)) {
        if (original_get_context) {
            return original_get_context(session_id, context);
        }
        return False;
    }

    // After a locally-skipped keyup, IME state is unchanged — return saved snapshot.
    if (g_last_was_keyup_skip && g_has_last_context) {
        return RestoreContextSnapshot(context);
    }

    LOG(INFO) << "[grpc_proxy] MyGetContext called(session=" << session_id << ")";

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
                context->composition.preedit = new char[comp.preedit().length() + 1];
                std::strcpy(context->composition.preedit, comp.preedit().c_str());
            }
        }
        
        if (proto.has_menu()) {
            const auto& menu = proto.menu();
            context->menu.page_size = menu.page_size();
            context->menu.page_no = menu.page_no();
            context->menu.is_last_page = menu.is_last_page() ? True : False;
            context->menu.highlighted_candidate_index = menu.highlighted_candidate_index();
            context->menu.num_candidates = menu.num_candidates();
            
            if (!menu.select_keys().empty() && RIME_STRUCT_HAS_MEMBER(*context, context->menu.select_keys)) {
                context->menu.select_keys = new char[menu.select_keys().length() + 1];
                std::strcpy(context->menu.select_keys, menu.select_keys().c_str());
            }
            
            if (menu.num_candidates() > 0) {
                context->menu.candidates = new RimeCandidate[menu.num_candidates()];
                for (int i = 0; i < menu.num_candidates(); ++i) {
                    const auto& cand = menu.candidates(i);
                    auto text = cand.text().empty() ? "" : cand.text().c_str();
                    auto comment = cand.comment().empty() ? "" : cand.comment().c_str();
                    
                    context->menu.candidates[i].text = new char[strlen(text) + 1];
                    std::strcpy(context->menu.candidates[i].text, text);
                    context->menu.candidates[i].comment = new char[strlen(comment) + 1];
                    std::strcpy(context->menu.candidates[i].comment, comment);
                }
            }
        }
        
        if (!proto.commit_text_preview().empty() && RIME_STRUCT_HAS_MEMBER(*context, context->commit_text_preview)) {
            context->commit_text_preview = new char[proto.commit_text_preview().length() + 1];
            std::strcpy(context->commit_text_preview, proto.commit_text_preview().c_str());
        }

        // Override select_keys based on v_mode_preedit_regex config
        if (context->composition.preedit && context->menu.page_size > 0) {
            auto cli = GrpcImeClientV2::Instance();
            if (cli && cli->HasVModeRegex()) {
                const char* alpha_keys = "abcdefghij";
                const char* num_keys   = "1234567890";
                bool is_vmode = cli->MatchesVMode(context->composition.preedit);
                const char* src_keys = is_vmode ? alpha_keys : num_keys;
                int n = context->menu.page_size;
                if (n > 10) n = 10;
                // Free previous allocation if any
                delete[] context->menu.select_keys;
                context->menu.select_keys = new char[n + 1];
                std::memcpy(context->menu.select_keys, src_keys, n);
                context->menu.select_keys[n] = '\0';
            }
        }
        
        // Save snapshot for future keyup-skip returns
        SaveContextSnapshot(context);
        return True;
    }
    return False;
}

static Bool MyGetStatus(RimeSessionId session_id, RIME_FLAVORED(RimeStatus)* status) {
    if (!status || status->data_size <= 0) return False;

    auto client = GrpcImeClientV2::Instance();
    if (!client || !client->HasSession(session_id)) {
        if (original_get_status) {
            return original_get_status(session_id, status);
        }
        return False;
    }

    // After keyup skip, derive from saved snapshot — no RPC needed.
    if (g_last_was_keyup_skip) {
        RIME_STRUCT_CLEAR(*status);
        status->is_composing = g_last_is_composing;
        
        char* proxy_id = new char[5];
        std::strcpy(proxy_id, "grpc");
        status->schema_id = proxy_id;

        char* proxy_name = new char[11];
        std::strcpy(proxy_name, "gRPC Proxy");
        status->schema_name = proxy_name;
        
        return True;
    }

    LOG(INFO) << "[grpc_proxy] MyGetStatus called(session=" << session_id << ")";

    service::v2::RimeContextProto proto;
    if (client->GetContext(session_id, &proto)) {
        RIME_STRUCT_CLEAR(*status);
        status->is_composing = proto.has_composition();
        
        char* proxy_id = new char[5];
        std::strcpy(proxy_id, "grpc");
        status->schema_id = proxy_id;

        char* proxy_name = new char[11];
        std::strcpy(proxy_name, "gRPC Proxy");
        status->schema_name = proxy_name;
        
        g_last_is_composing = status->is_composing;
        return True;
    }
    return False;
}

static Bool MyGetCommit(RimeSessionId session_id, RIME_FLAVORED(RimeCommit)* commit) {
    if (!commit || commit->data_size <= 0) return False;

    auto client = GrpcImeClientV2::Instance();
    if (!client || !client->HasSession(session_id)) {
        if (original_get_commit) {
            return original_get_commit(session_id, commit);
        }
        return False;
    }

    // After keyup skip or when ProcessKey returned false, no new commit is possible.
    if (g_last_was_keyup_skip || !g_last_process_key_accepted) {
        return False;
    }

    LOG(INFO) << "[grpc_proxy] MyGetCommit called(session=" << session_id << ")";

    std::string text;
    if (client->GetCommit(session_id, &text) && !text.empty()) {
        RIME_STRUCT_CLEAR(*commit);
        commit->text = new char[text.length() + 1];
        std::strcpy(commit->text, text.c_str());
        return True;
    }
    return False;
}

static Bool MySelectCandidate(RimeSessionId session_id, size_t index) {
    LOG(INFO) << "[grpc_proxy] MySelectCandidate called(session=" << session_id << ", index=" << index << ")";
    auto client = GrpcImeClientV2::Instance();
    if (client) {
        return client->SelectCandidate(session_id, index) ? True : False;
    }
    return False;
}

static Bool MySelectCandidateOnCurrentPage(RimeSessionId session_id, size_t index) {
    LOG(INFO) << "[grpc_proxy] MySelectCandidateOnCurrentPage called(session=" << session_id << ", index=" << index << ")";
    auto client = GrpcImeClientV2::Instance();
    if (client) {
        return client->SelectCandidateOnCurrentPage(session_id, index) ? True : False;
    }
    return False;
}

static void rime_grpc_proxy_v2_initialize() {
  LOG(INFO) << "[grpc_proxy] rime_grpc_proxy_v2_initialize called!";
  RimeApi* api = const_cast<RimeApi*>(rime_get_api());
  if (api) {
      original_create_session = api->create_session;
      original_destroy_session = api->destroy_session;
      original_find_session = api->find_session;
      original_process_key = api->process_key;
      original_simulate_key_sequence = api->simulate_key_sequence;
      original_get_context = api->get_context;
      original_get_status = api->get_status;
      original_get_commit = api->get_commit;
      original_select_candidate = api->select_candidate;
      original_select_candidate_on_current_page = api->select_candidate_on_current_page;

      api->create_session = MyCreateSession;
      api->destroy_session = MyDestroySession;
      api->find_session = MyFindSession;
      api->process_key = MyProcessKey;
      api->simulate_key_sequence = MySimulateKeySequence;
      api->get_context = MyGetContext;
      api->get_status = MyGetStatus;
      api->get_commit = MyGetCommit;
      api->select_candidate = MySelectCandidate;
      api->select_candidate_on_current_page = MySelectCandidateOnCurrentPage;
      LOG(INFO) << "[grpc_proxy] RimeApi successfully overridden!";
  } else {
      LOG(ERROR) << "[grpc_proxy] rime_get_api() returned NULL!";
  }
}

static void rime_grpc_proxy_v2_finalize() {}

RIME_REGISTER_MODULE(grpc_proxy_v2)
