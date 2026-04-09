#include <rime/component.h>
#include <rime/registry.h>
#include <rime_api.h>

#include "win32_proxy_translator.h"

using namespace rime;

static void rime_win32_proxy_initialize() {
  Registry& r = Registry::instance();
  r.Register("win32_proxy_translator",
             new Component<win32_proxy::Win32ProxyTranslator>);
}

static void rime_win32_proxy_finalize() {}

RIME_REGISTER_MODULE(win32_proxy)
