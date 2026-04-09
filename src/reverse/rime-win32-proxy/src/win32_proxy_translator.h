#ifndef RIME_WIN32_PROXY_TRANSLATOR_H_
#define RIME_WIN32_PROXY_TRANSLATOR_H_

#include <rime/common.h>
#include <rime/translator.h>

namespace rime {
namespace win32_proxy {

class Win32ProxyTranslator : public Translator {
 public:
  explicit Win32ProxyTranslator(const Ticket& ticket);

  an<Translation> Query(const string& input, const Segment& segment) override;

 private:
  class Client;
  the<Client> client_;

  string tag_;
  int max_candidates_ = 9;
  string comment_mode_;
};

}  // namespace win32_proxy
}  // namespace rime

#endif  // RIME_WIN32_PROXY_TRANSLATOR_H_
