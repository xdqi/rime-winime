#ifndef RIME_REMOTE_TRANSLATOR_H_
#define RIME_REMOTE_TRANSLATOR_H_

#include <rime/translator.h>
#include "shared_state.h"

namespace rime {

class RemoteTranslator : public Translator {
 public:
  explicit RemoteTranslator(const Ticket& ticket);

  an<Translation> Query(const string& input,
                        const Segment& segment) override;

 private:
  an<RemoteSharedState> state_;
};

}  // namespace rime

#endif  // RIME_REMOTE_TRANSLATOR_H_
