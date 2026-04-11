#ifndef RIME_REMOTE_PROCESSOR_H_
#define RIME_REMOTE_PROCESSOR_H_

#include <rime/processor.h>
#include "shared_state.h"

namespace rime {

class RemoteProcessor : public Processor {
 public:
  explicit RemoteProcessor(const Ticket& ticket);
  ~RemoteProcessor() override;

  ProcessResult ProcessKeyEvent(const KeyEvent& key_event) override;

 private:
  // Fetch gRPC context and store into shared state + engine context.
  void SyncContextFromBackend();
  // Fetch gRPC commit and call engine->CommitText().
  void SyncCommitFromBackend();
  // Handle ascii_mode toggle with active composition.
  void HandleAsciiModeToggle(int keycode);

  an<RemoteSharedState> state_;
  bool ascii_was_on_ = false;
};

}  // namespace rime

#endif  // RIME_REMOTE_PROCESSOR_H_
