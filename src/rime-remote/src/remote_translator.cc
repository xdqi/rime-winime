#include "remote_translator.h"
#include <rime/candidate.h>
#include <rime/engine.h>
#include <rime/segmentation.h>
#include <rime/translation.h>

namespace rime {

// A Translation that wraps the candidates snapshot from gRPC.
class RemoteTranslation : public Translation {
 public:
  RemoteTranslation(const RemoteContext& rc, size_t start, size_t end)
      : rc_(rc), start_(start), end_(end) {
    set_exhausted(rc_.candidates.empty());
  }

  bool Next() override {
    if (exhausted())
      return false;
    ++cursor_;
    if (cursor_ >= rc_.candidates.size())
      set_exhausted(true);
    return true;
  }

  an<Candidate> Peek() override {
    if (cursor_ >= rc_.candidates.size())
      return nullptr;
    const auto& c = rc_.candidates[cursor_];
    // Don't set preedit on the candidate — let GetPreedit() fall back to
    // displaying the raw input from the Segmentation.  This makes cursor
    // positioning work correctly via rime's native caret_pos mechanism.
    auto cand = New<SimpleCandidate>(
        "remote", start_, end_, c.text, c.comment);
    return cand;
  }

 private:
  RemoteContext rc_;  // copy — stable snapshot
  size_t start_;
  size_t end_;
  size_t cursor_ = 0;
};

RemoteTranslator::RemoteTranslator(const Ticket& ticket) : Translator(ticket) {
  state_ = RemoteStateRegistry::instance().Get(engine_);
}

an<Translation> RemoteTranslator::Query(const string& input,
                                        const Segment& segment) {
  if (!state_ || !state_->has_context())
    return nullptr;

  if (!segment.HasTag("remote"))
    return nullptr;

  const auto& rc = state_->context();
  if (rc.candidates.empty())
    return nullptr;

  return New<RemoteTranslation>(rc, segment.start, segment.end);
}

}  // namespace rime
