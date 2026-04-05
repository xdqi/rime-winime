#include <rime/component.h>
#include <rime/registry.h>
#include <rime_api.h>
#include <cstring>

#include "grpc_key_event_processor.h"
#include "grpc_client.h"

using namespace rime;

static RimeSessionId (*original_create_session)();
static Bool (*original_destroy_session)(RimeSessionId);
static Bool (*original_find_session)(RimeSessionId);
static Bool (*original_process_key)(RimeSessionId, int, int);
static Bool (*original_get_context)(RimeSessionId, RIME_FLAVORED(RimeContext)*);
static Bool (*original_get_status)(RimeSessionId, RIME_FLAVORED(RimeStatus)*);
static Bool (*original_get_commit)(RimeSessionId, RIME_FLAVORED(RimeCommit)*);

static RimeSessionId MyCreateSession() {
    auto client = GrpcImeClientV2::Instance();
    if (client) {
        return client->OpenSession();
    }
    return 0;
}

static Bool MyDestroySession(RimeSessionId session_id) {
    auto client = GrpcImeClientV2::Instance();
    if (client) {
        client->DestroySession(session_id);
        return True;
    }
    return False;
}

static Bool MyFindSession(RimeSessionId session_id) {
    // If it's non-zero we just consider it found for basic tests, ideally check if it exists in the map
    return session_id != 0 ? True : False;
}

static Bool MyProcessKey(RimeSessionId session_id, int keycode, int mask) {
    auto client = GrpcImeClientV2::Instance();
    if (client) {
        return client->ProcessKey(session_id, keycode, mask);
    }
    return False;
}

static Bool MyGetContext(RimeSessionId session_id, RIME_FLAVORED(RimeContext)* context) {
    if (!context || context->data_size <= 0) return False;
    
    auto client = GrpcImeClientV2::Instance();
    if (!client) return False;
    
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
        
        return True;
    }
    return False;
}

static Bool MyGetStatus(RimeSessionId session_id, RIME_FLAVORED(RimeStatus)* status) {
    if (!status || status->data_size <= 0) return False;
    
    auto client = GrpcImeClientV2::Instance();
    if (!client) return False;

    service::v2::RimeContextProto proto;
    if (client->GetContext(session_id, &proto)) {
        RIME_STRUCT_CLEAR(*status);
        status->is_composing = proto.has_composition();
        return True;
    }
    return False;
}

static Bool MyGetCommit(RimeSessionId session_id, RIME_FLAVORED(RimeCommit)* commit) {
    if (!commit || commit->data_size <= 0) return False;
    
    auto client = GrpcImeClientV2::Instance();
    if (!client) return False;
    
    std::string text;
    if (client->GetCommit(session_id, &text) && !text.empty()) {
        RIME_STRUCT_CLEAR(*commit);
        commit->text = new char[text.length() + 1];
        std::strcpy(commit->text, text.c_str());
        return True;
    }
    return False;
}

static void rime_grpc_proxy_v2_initialize() {
  RimeApi* api = const_cast<RimeApi*>(rime_get_api());
  if (api) {
      original_create_session = api->create_session;
      original_destroy_session = api->destroy_session;
      original_find_session = api->find_session;
      original_process_key = api->process_key;
      original_get_context = api->get_context;
      original_get_status = api->get_status;
      original_get_commit = api->get_commit;

      api->create_session = MyCreateSession;
      api->destroy_session = MyDestroySession;
      api->find_session = MyFindSession;
      api->process_key = MyProcessKey;
      api->get_context = MyGetContext;
      api->get_status = MyGetStatus;
      api->get_commit = MyGetCommit;
  }
}

static void rime_grpc_proxy_v2_finalize() {}

RIME_REGISTER_MODULE(grpc_proxy_v2)
