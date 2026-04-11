#ifndef RIME_REMOTE_SEGMENTOR_H_
#define RIME_REMOTE_SEGMENTOR_H_

#include <rime/segmentor.h>
#include "shared_state.h"

namespace rime {

class RemoteSegmentor : public Segmentor {
 public:
  explicit RemoteSegmentor(const Ticket& ticket);

  bool Proceed(Segmentation* segmentation) override;

 private:
  an<RemoteSharedState> state_;
};

}  // namespace rime

#endif  // RIME_REMOTE_SEGMENTOR_H_
