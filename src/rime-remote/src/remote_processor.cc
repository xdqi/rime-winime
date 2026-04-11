// grpc/abseil headers first — see grpc_proxy_module.cc for rationale.
#include "grpc_client.h"

#include "remote_processor.h"
#include <rime/config.h>
#include <rime/context.h>
#include <rime/engine.h>
#include <rime/key_event.h>
#include <rime/schema.h>
#include <rime/service.h>

namespace rime {

// Modifier-only keycodes (XK_Shift_L through XK_Hyper_R).
static bool IsModifierOnlyKey(int keycode) {
  return (keycode >= 0xFFE1 && keycode <= 0xFFEE);
}

static RemoteSharedState::AsciiSwitchStyle ParseSwitchStyle(
    const std::string& s) {
  if (s == "inline_ascii") return RemoteSharedState::kSwitchInlineAscii;
  if (s == "commit_text")  return RemoteSharedState::kSwitchCommitText;
  if (s == "commit_code")  return RemoteSharedState::kSwitchCommitCode;
  if (s == "clear")        return RemoteSharedState::kSwitchClear;
  return RemoteSharedState::kSwitchNoop;
}

RemoteProcessor::RemoteProcessor(const Ticket& ticket) : Processor(ticket) {
  state_ = New<RemoteSharedState>();

  // Read config from schema.
  std::string backend_address = "127.0.0.1:50051";
  std::string backend_schema_id = "luna_pinyin";
  int rpc_timeout_ms = 200;
  std::string v_mode_preedit_regex;

  if (auto* config = engine_->schema()->config()) {
    config->GetString("remote/backend_address", &backend_address);
    config->GetString("remote/backend_schema_id", &backend_schema_id);
    config->GetInt("remote/rpc_timeout_ms", &rpc_timeout_ms);
    config->GetString("remote/v_mode_preedit_regex", &v_mode_preedit_regex);

    // Read ascii_composer/switch_key bindings.
    static const struct { const char* name; int keycode; } key_names[] = {
      {"Shift_L",   0xFFE1}, {"Shift_R",   0xFFE2},
      {"Control_L", 0xFFE3}, {"Control_R", 0xFFE4},
      {"Caps_Lock", 0xFFE5}, {"Eisu_toggle", 0xFF30},
    };
    for (const auto& kn : key_names) {
      std::string style_str;
      std::string path = std::string("ascii_composer/switch_key/") + kn.name;
      if (config->GetString(path, &style_str)) {
        auto style = ParseSwitchStyle(style_str);
        state_->switch_key_bindings[kn.keycode] = style;
      }
    }
  }

  // Create gRPC client.
  state_->client = GrpcImeClientV2::GetOrCreate(backend_address, rpc_timeout_ms);
  state_->backend_address = backend_address;
  if (state_->client && !v_mode_preedit_regex.empty()) {
    state_->client->SetVModeRegex(v_mode_preedit_regex);
  }

  // Open gRPC session (skip if already opened for this engine, e.g. after
  // ApplySchema re-creates components).
  state_->session_id = reinterpret_cast<uintptr_t>(engine_);
  if (state_->client && !state_->client->HasSession(state_->session_id)) {
    if (state_->client->OpenSession(state_->session_id, backend_schema_id)) {
      state_->grpc_session_open = true;
      LOG(INFO) << "[remote] opened gRPC session for engine " << state_->session_id
                << " schema=" << backend_schema_id;
    }
  } else if (state_->client) {
    state_->grpc_session_open = true;
    LOG(INFO) << "[remote] reusing existing gRPC session for engine " << state_->session_id;
  }

  // Register shared state so Segmentor/Translator can find it.
  RemoteStateRegistry::instance().Register(engine_, state_);

  // Track initial ascii_mode.
  ascii_was_on_ = engine_->context()->get_option("ascii_mode");
}

RemoteProcessor::~RemoteProcessor() {
  RemoteStateRegistry::instance().Unregister(engine_);
  if (state_->grpc_session_open && state_->client) {
    state_->client->DestroySession(state_->session_id);
    state_->grpc_session_open = false;
  }
}

ProcessResult RemoteProcessor::ProcessKeyEvent(const KeyEvent& key_event) {
  if (!state_->grpc_session_open || !state_->client)
    return kNoop;

  int keycode = key_event.keycode();
  int mask = key_event.modifier();
  Context* ctx = engine_->context();

  // ascii_composer runs before us and may have already toggled the option.
  // Compare current state with our saved snapshot to detect toggles.
  bool ascii_now = ctx->get_option("ascii_mode");

  // Detect ascii_mode toggle (e.g. Shift tap via ascii_composer).
  if (ascii_now != ascii_was_on_) {
    if (ascii_now && state_->has_context()) {
      HandleAsciiModeToggle(keycode);
    }
    ascii_was_on_ = ascii_now;
    // Don't forward this key — it was a mode-switch key.
    return kNoop;
  }

  // Key release — skip RPC round-trip.
  if (key_event.release())
    return kNoop;

  // Modifier-only keys — don't forward to gRPC.
  if (IsModifierOnlyKey(keycode))
    return kNoop;

  // If ascii_mode is ON, let keys pass through as English.
  if (ascii_now) {
    return kNoop;
  }

  // Forward to gRPC backend.
  // Save old preedit to detect if context changed (handles timeout case).
  std::string old_preedit;
  if (state_->has_context()) {
    old_preedit = state_->context().preedit;
  }

  bool accepted = state_->client->ProcessKey(
      state_->session_id, keycode, mask);

  LOG(INFO) << "[remote] ProcessKey(key=" << keycode
            << ", mask=" << mask << ") -> " << accepted;

  // Always sync state — even if ProcessKey timed out, the backend may have
  // processed the key, and GetContext/GetCommit will reflect that.
  SyncCommitFromBackend();
  SyncContextFromBackend();

  // Detect if the context actually changed (handles RPC timeout: ProcessKey
  // returns false but the backend did process the key).
  bool context_changed = false;
  if (state_->has_context()) {
    context_changed = (state_->context().preedit != old_preedit);
  } else {
    context_changed = !old_preedit.empty();  // had context before, now gone
  }

  if (!accepted && !context_changed) {
    // Backend truly rejected the key and context didn't change.
    // Let the key pass through to other processors.
    return kNoop;
  }

  // Backend accepted (or timed out but context changed).  Treat as accepted.

  // If context has composition, inject input to trigger Compose pipeline.
  if (state_->has_context()) {
    const auto& rc = state_->context();
    // Use the actual preedit text from the backend as input.
    // This ensures:
    //  - commit_code (ascii_composer) commits the real preedit, not garbage
    //  - cursor positioning works via rime's native caret_pos mechanism
    //  - the Candidate does NOT override preedit(), so GetPreedit() falls
    //    back to displaying the raw input string from the Segmentation.
    const std::string& input_text =
        rc.preedit.empty() ? std::string(1, ' ') : rc.preedit;

    // Clear the old composition before setting new input.
    // Without this, Segmentation::Reset's prefix-matching optimization keeps
    // stale segments whose end <= diff_pos, causing split segmentation
    // (e.g. "ss"→"ssh" keeps old [0,2) and only adds [2,3) instead of [0,3)).
    // Clear() fires update_notifier on empty input, which Compose ignores.
    if (ctx->IsComposing()) {
      ctx->Clear();
    }

    // set_input triggers update_notifier → Compose → Segmentor → Translator.
    // set_input always resets caret to end; set_caret_pos corrects it and
    // triggers a second Compose only if the cursor is not at the end.
    ctx->set_input(input_text);
    if (rc.cursor_pos >= 0 &&
        static_cast<size_t>(rc.cursor_pos) < input_text.length()) {
      ctx->set_caret_pos(static_cast<size_t>(rc.cursor_pos));
    }
  } else {
    // Backend accepted but no composition (e.g. committed everything).
    if (ctx->IsComposing()) {
      ctx->Clear();
    }
  }

  return kAccepted;
}

void RemoteProcessor::SyncContextFromBackend() {
  service::v2::RimeContextProto proto;
  if (!state_->client->GetContext(state_->session_id, &proto)) {
    state_->ClearContext();
    return;
  }

  RemoteContext rc;
  if (proto.has_composition()) {
    const auto& comp = proto.composition();
    rc.preedit = comp.preedit();
    rc.cursor_pos = comp.cursor_pos();
    rc.sel_start = comp.sel_start();
    rc.sel_end = comp.sel_end();
    DLOG(INFO) << "[remote] context: preedit=\"" << rc.preedit
               << "\" cursor=" << rc.cursor_pos;
  }
  if (proto.has_menu()) {
    const auto& menu = proto.menu();
    rc.page_size = menu.page_size();
    rc.page_no = menu.page_no();
    rc.is_last_page = menu.is_last_page();
    rc.highlighted_candidate_index = menu.highlighted_candidate_index();
    rc.select_keys = menu.select_keys();
    rc.candidates.reserve(menu.num_candidates());
    for (int i = 0; i < menu.num_candidates(); ++i) {
      const auto& c = menu.candidates(i);
      rc.candidates.push_back({c.text(), c.comment()});
    }
  }
  rc.commit_text_preview = proto.commit_text_preview();

  // Apply v_mode select_keys override.
  if (!rc.preedit.empty() && rc.page_size > 0 &&
      state_->client->HasVModeRegex()) {
    bool is_vmode = state_->client->MatchesVMode(rc.preedit);
    const char* alpha_keys = "abcdefghij";
    const char* num_keys   = "1234567890";
    const char* src = is_vmode ? alpha_keys : num_keys;
    int n = rc.page_size > 10 ? 10 : rc.page_size;
    rc.select_keys.assign(src, n);
  }

  if (rc.preedit.empty() && rc.candidates.empty()) {
    state_->ClearContext();
  } else {
    state_->SetContext(std::move(rc));
  }
}

void RemoteProcessor::SyncCommitFromBackend() {
  std::string commit_text;
  if (state_->client->GetCommit(state_->session_id, &commit_text) &&
      !commit_text.empty()) {
    engine_->CommitText(commit_text);
    LOG(INFO) << "[remote] committed: " << commit_text;
  }
}

void RemoteProcessor::HandleAsciiModeToggle(int keycode) {
  Context* ctx = engine_->context();

  // Determine which switch style this key uses.
  auto style = RemoteSharedState::kSwitchNoop;
  auto it = state_->switch_key_bindings.find(keycode);
  if (it != state_->switch_key_bindings.end()) {
    style = it->second;
  }

  if (state_->has_context()) {
    const auto& rc = state_->context();
    if (style == RemoteSharedState::kSwitchCommitText) {
      // commit_text: commit the first candidate (the translated text).
      // ascii_composer called ConfirmCurrentSelection() which doesn't auto-
      // commit without _auto_commit option.  We handle it here instead.
      if (!rc.candidates.empty()) {
        // Clear whatever ascii_composer left in the composition.
        ctx->Clear();
        engine_->CommitText(rc.candidates[0].text);
        LOG(INFO) << "[remote] commit_text: " << rc.candidates[0].text;
      }
    } else if (style == RemoteSharedState::kSwitchCommitCode) {
      // commit_code: ascii_composer already committed the raw input (the
      // preedit text we set via set_input).  Nothing extra to do.
    }
    // For kSwitchClear or kSwitchNoop: ascii_composer already handled it.
  }

  // Reset the gRPC backend's composition so it doesn't carry stale state.
  if (state_->grpc_session_open && state_->client) {
    state_->client->ProcessKey(state_->session_id, 0xFF1B /* Escape */, 0);
    // Drain any pending commit produced by the Escape.
    std::string discard;
    state_->client->GetCommit(state_->session_id, &discard);
  }
  state_->ClearContext();
}

}  // namespace rime
