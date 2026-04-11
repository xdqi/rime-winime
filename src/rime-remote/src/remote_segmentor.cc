#include "remote_segmentor.h"
#include <rime/engine.h>
#include <rime/segmentation.h>

namespace rime {

RemoteSegmentor::RemoteSegmentor(const Ticket& ticket) : Segmentor(ticket) {
  state_ = RemoteStateRegistry::instance().Get(engine_);
}

bool RemoteSegmentor::Proceed(Segmentation* segmentation) {
  if (!state_ || !state_->has_context())
    return true;  // no remote data — let other segmentors handle it

  // Create a single segment covering the entire input, tagged "remote".
  size_t start = segmentation->GetCurrentStartPosition();
  size_t end = segmentation->input().length();
  if (end <= start)
    return true;

  Segment segment(static_cast<int>(start), static_cast<int>(end));
  segment.tags.insert("remote");
  segmentation->AddSegment(segment);

  return false;  // we claimed the whole input — stop other segmentors
}

}  // namespace rime
