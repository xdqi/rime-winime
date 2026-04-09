#include "win32_proxy_translator.h"

#include <arpa/inet.h>
#include <errno.h>
#include <netinet/in.h>
#include <sys/select.h>
#include <sys/socket.h>
#include <sys/types.h>
#include <unistd.h>

#include <algorithm>
#include <cctype>
#include <cstring>
#include <mutex>
#include <sstream>
#include <string>
#include <vector>

#include <rime/candidate.h>
#include <rime/config.h>
#include <rime/schema.h>
#include <rime/segmentation.h>
#include <rime/translation.h>

#ifndef MSG_NOSIGNAL
#define MSG_NOSIGNAL 0
#endif

namespace rime {
namespace win32_proxy {

namespace {

struct ProxyResult {
  string composition;
  string reading;
  vector<string> items;
};

static bool starts_with(const string& text, const string& prefix) {
  return text.rfind(prefix, 0) == 0;
}

static bool is_connect_pending_errno(int err) {
  return err == EINPROGRESS || err == EALREADY || err == EWOULDBLOCK ||
         err == EINTR;
}

static bool extract_between(const string& text,
                            const string& begin,
                            char end_char,
                            string* out,
                            size_t from = 0) {
  size_t p = text.find(begin, from);
  if (p == string::npos) {
    return false;
  }
  p += begin.size();
  size_t q = text.find(end_char, p);
  if (q == string::npos) {
    return false;
  }
  *out = text.substr(p, q - p);
  return true;
}

static vector<string> split_items(const string& text) {
  vector<string> items;
  std::stringstream ss(text);
  string token;

  while (std::getline(ss, token, '|')) {
    if (token.empty() || token == "...") {
      continue;
    }
    items.push_back(token);
  }

  return items;
}

static string sanitize_line(string s) {
  string out;
  out.reserve(s.size());
  for (unsigned char c : s) {
    if (c == '\r' || c == '\n') {
      out.push_back(' ');
      continue;
    }
    if (c < 0x20 || c == 0x7f) {
      continue;
    }

    // Filter out terminal redraw debris (e.g. "[K") that may leak in when
    // users edit input with backspace in readline.
    if (c < 0x80) {
      if (std::isalnum(c) || c == '\'' || c == ' ' || c == '_' || c == '-') {
        out.push_back(static_cast<char>(c));
      }
      continue;
    }

    out.push_back(static_cast<char>(c));
  }
  return out;
}

static string sanitize_query_input(string s) {
  string out;
  out.reserve(s.size());
  for (unsigned char c : s) {
    if (c == '\r' || c == '\n' || c == '\t') {
      continue;
    }
    if (c < 0x20 || c == 0x7f) {
      continue;
    }

    // Query input is pinyin-oriented. Keep only lowercase ascii letters,
    // digits and apostrophe to avoid terminal redraw artifacts leaking in.
    if (c < 0x80) {
      if ((c >= 'a' && c <= 'z') || (c >= '0' && c <= '9') || c == '\'') {
        out.push_back(static_cast<char>(c));
      }
      continue;
    }

    out.push_back(static_cast<char>(c));
  }
  return out;
}

static bool parse_candidate_reply(const string& line, ProxyResult* result) {
  if (!starts_with(line, "CAND_RET ")) {
    return false;
  }

  if (line.find("err=") != string::npos) {
    return false;
  }

  (void)extract_between(line, "comp=[", ']', &result->composition);
  (void)extract_between(line, "read=[", ']', &result->reading);

  size_t p0 = line.find("#0{");
  if (p0 == string::npos) {
    return true;
  }

  string raw_items;
  if (!extract_between(line, "items=[", ']', &raw_items, p0)) {
    return true;
  }

  result->items = split_items(raw_items);
  return true;
}

}  // namespace

class Win32ProxyTranslator::Client {
 public:
  Client(string host,
         int port,
         int codepage,
         int timeout_ms,
         string command,
         bool one_shot)
      : host_(std::move(host)),
        port_(port),
        codepage_(codepage),
        timeout_ms_(timeout_ms),
        command_(std::move(command)),
        one_shot_(one_shot) {}

  ~Client() {
    std::lock_guard<std::mutex> lock(mu_);
    close_unlocked();
  }

  bool Query(const string& utf8_input, ProxyResult* out) {
    std::lock_guard<std::mutex> lock(mu_);

    const string input = sanitize_query_input(utf8_input);

    auto run_candidate_command = [&](const string& cmd,
                                     ProxyResult* parsed,
                                     string* raw_reply) -> bool {
      string response;
      if (!send_command_unlocked(cmd + " " + input, &response,
                                 "CAND_RET ")) {
        return false;
      }
      if (raw_reply != nullptr) {
        *raw_reply = response;
      }
      if (!parse_candidate_reply(response, parsed)) {
        return false;
      }
      return true;
    };

    for (int attempt = 0; attempt < 2; ++attempt) {
      if (!ensure_connected_unlocked()) {
        close_unlocked();
        continue;
      }

      string ignored;
      if (!send_command_unlocked("ACTIVATE", &ignored, "OK ACTIVATE")) {
        close_unlocked();
        continue;
      }

      ProxyResult parsed;
      string response;
      if (!run_candidate_command(command_, &parsed, &response)) {
        DLOG(WARNING) << "win32_proxy: bad reply: " << response;
        close_unlocked();
        continue;
      }

      if (parsed.items.empty()) {
        string cand_reply;
        ProxyResult fallback;
        if (send_command_unlocked("CAND", &cand_reply, "CAND_RET ") &&
            parse_candidate_reply(cand_reply, &fallback) &&
            !fallback.items.empty()) {
          parsed = std::move(fallback);
        }
      }

      if (parsed.items.empty() && command_ != "KEYTEXTU") {
        ProxyResult fallback;
        if (run_candidate_command("KEYTEXTU", &fallback, nullptr) &&
            !fallback.items.empty()) {
          parsed = std::move(fallback);
        }
      }

      if (parsed.items.empty() && command_ != "TEXTU") {
        ProxyResult fallback;
        if (run_candidate_command("TEXTU", &fallback, nullptr) &&
            !fallback.items.empty()) {
          parsed = std::move(fallback);
        }
      }

      if (!parsed.items.empty()) {
        *out = std::move(parsed);
        if (one_shot_) {
          close_unlocked();
        }
        return true;
      }

      DLOG(INFO) << "win32_proxy: empty candidates for input='" << utf8_input
                 << "' command=" << command_ << " attempt=" << attempt;

      (void)send_command_unlocked("RESET", &ignored, "OK RESET");
      close_unlocked();
    }

    return false;
  }

 private:
  bool ensure_connected_unlocked() {
    if (fd_ >= 0) {
      return true;
    }

    fd_ = ::socket(AF_INET, SOCK_STREAM, 0);
    if (fd_ < 0) {
      LOG(ERROR) << "win32_proxy: socket() failed: errno=" << errno;
      return false;
    }

    if (!set_timeout_unlocked(fd_, timeout_ms_)) {
      LOG(WARNING) << "win32_proxy: failed to set socket timeout";
    }

    sockaddr_in addr;
    std::memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_port = htons(static_cast<uint16_t>(port_));

    if (::inet_pton(AF_INET, host_.c_str(), &addr.sin_addr) != 1) {
      LOG(ERROR) << "win32_proxy: bad host address: " << host_;
      close_unlocked();
      return false;
    }

    if (::connect(fd_, reinterpret_cast<const sockaddr*>(&addr), sizeof(addr)) !=
        0) {
      int err = errno;
      if (is_connect_pending_errno(err)) {
        bool connected = false;
        int remaining_ms = timeout_ms_;

        while (remaining_ms > 0) {
          const int slice_ms = std::min(remaining_ms, 300);

          fd_set wfds;
          FD_ZERO(&wfds);
          FD_SET(fd_, &wfds);

          timeval tv;
          tv.tv_sec = slice_ms / 1000;
          tv.tv_usec = (slice_ms % 1000) * 1000;

          int sel = ::select(fd_ + 1, nullptr, &wfds, nullptr, &tv);
          if (sel < 0) {
            if (errno == EINTR) {
              continue;
            }
            LOG(ERROR) << "win32_proxy: connect wait failed to " << host_
                       << ":" << port_ << " errno=" << errno;
            close_unlocked();
            return false;
          }

          if (sel == 0) {
            remaining_ms -= slice_ms;
            continue;
          }

          int so_error = 0;
          socklen_t so_len = sizeof(so_error);
          if (::getsockopt(fd_, SOL_SOCKET, SO_ERROR, &so_error, &so_len) != 0) {
            LOG(ERROR) << "win32_proxy: getsockopt(SO_ERROR) failed errno="
                       << errno;
            close_unlocked();
            return false;
          }

          if (so_error == 0) {
            connected = true;
            break;
          }

          if (is_connect_pending_errno(so_error)) {
            remaining_ms -= slice_ms;
            continue;
          }

          LOG(ERROR) << "win32_proxy: connect failed to " << host_ << ":"
                     << port_ << " errno=" << so_error;
          close_unlocked();
          return false;
        }

        if (!connected) {
          LOG(ERROR) << "win32_proxy: connect timeout to " << host_ << ":"
                     << port_ << " errno=" << err;
          close_unlocked();
          return false;
        }
      } else {
        LOG(ERROR) << "win32_proxy: connect failed to " << host_ << ":"
                   << port_ << " errno=" << err;
        close_unlocked();
        return false;
      }
    }

    string ignored;
    if (!send_command_unlocked("ACTIVATE", &ignored, "OK ACTIVATE")) {
      LOG(ERROR) << "win32_proxy: ACTIVATE failed";
      close_unlocked();
      return false;
    }

    if (!send_command_unlocked("CP " + std::to_string(codepage_), &ignored,
                               "OK CP ")) {
      LOG(ERROR) << "win32_proxy: CP command failed";
      close_unlocked();
      return false;
    }

    return true;
  }

  bool set_timeout_unlocked(int fd, int timeout_ms) {
    timeval tv;
    tv.tv_sec = timeout_ms / 1000;
    tv.tv_usec = (timeout_ms % 1000) * 1000;

    return ::setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)) == 0 &&
           ::setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv)) == 0;
  }

  bool send_command_unlocked(const string& cmd,
                             string* response,
                             const char* expected_prefix = nullptr) {
    string wire = cmd;
    wire.push_back('\n');

    size_t sent = 0;
    while (sent < wire.size()) {
      ssize_t n = ::send(fd_, wire.data() + sent, wire.size() - sent,
                         MSG_NOSIGNAL);
      if (n <= 0) {
        return false;
      }
      sent += static_cast<size_t>(n);
    }

    string line;
    string last_seen;
    for (int i = 0; i < 8; ++i) {
      if (!read_line_unlocked(&line)) {
        if (response != nullptr) {
          *response = last_seen;
        }
        return false;
      }
      last_seen = line;

      if (starts_with(line, "HELLO ")) {
        continue;
      }
      if (line.empty()) {
        continue;
      }
      if (expected_prefix == nullptr ||
          starts_with(line, expected_prefix)) {
        if (response != nullptr) {
          *response = line;
        }
        return true;
      }
    }

    if (response != nullptr) {
      *response = last_seen;
    }
    return false;
  }

  bool read_line_unlocked(string* line) {
    while (true) {
      size_t nl = rx_.find('\n');
      if (nl != string::npos) {
        *line = rx_.substr(0, nl);
        rx_.erase(0, nl + 1);
        if (!line->empty() && line->back() == '\r') {
          line->pop_back();
        }
        return true;
      }

      char buf[512];
      ssize_t n = ::recv(fd_, buf, sizeof(buf), 0);
      if (n <= 0) {
        return false;
      }
      rx_.append(buf, static_cast<size_t>(n));
    }
  }

  void close_unlocked() {
    if (fd_ >= 0) {
      ::close(fd_);
      fd_ = -1;
    }
    rx_.clear();
  }

  std::mutex mu_;
  int fd_ = -1;
  string rx_;

  string host_;
  int port_ = 22345;
  int codepage_ = 936;
  int timeout_ms_ = 2500;
  string command_ = "PIPEU";
  bool one_shot_ = false;
};

Win32ProxyTranslator::Win32ProxyTranslator(const Ticket& ticket)
    : Translator(ticket) {
  string config_ns = "win32_proxy";
  if (name_space_.empty()) {
    name_space_ = "win32_proxy";
  } else if (name_space_ != "translator") {
    config_ns = name_space_;
  }

  string host = "127.0.0.1";
  int port = 22345;
  int codepage = 936;
  int timeout_ms = 2500;
  string command = "PIPEU";
  bool one_shot = false;
  tag_ = "abc";
  comment_mode_ = "none";

  if (ticket.schema) {
    if (Config* config = ticket.schema->config()) {
      (void)config->GetString(config_ns + "/host", &host);
      (void)config->GetInt(config_ns + "/port", &port);
      (void)config->GetInt(config_ns + "/codepage", &codepage);
      (void)config->GetInt(config_ns + "/timeout_ms", &timeout_ms);
      (void)config->GetString(config_ns + "/command", &command);
      (void)config->GetString(config_ns + "/tag", &tag_);
      (void)config->GetInt(config_ns + "/max_candidates", &max_candidates_);
      (void)config->GetString(config_ns + "/comment_mode", &comment_mode_);
      (void)config->GetBool(config_ns + "/one_shot", &one_shot);
    }
  }

  if (max_candidates_ <= 0) {
    max_candidates_ = 9;
  }

  if (command.empty()) {
    command = "PIPEU";
  }

  command = sanitize_line(command);

  client_.reset(new Client(host, port, codepage, timeout_ms, command, one_shot));

  LOG(INFO) << "win32_proxy translator initialized: ns=" << name_space_
            << " cfg=" << config_ns
            << " host=" << host << ":" << port
            << " command=" << command << " tag=" << tag_;
}

an<Translation> Win32ProxyTranslator::Query(const string& input,
                                            const Segment& segment) {
  if (input.empty()) {
    return nullptr;
  }

  if (!tag_.empty() && !segment.HasTag(tag_)) {
    return nullptr;
  }

  ProxyResult result;
  if (!client_ || !client_->Query(input, &result) || result.items.empty()) {
    return nullptr;
  }

  auto translation = New<FifoTranslation>();
  size_t count = std::min(result.items.size(), static_cast<size_t>(max_candidates_));

  for (size_t i = 0; i < count; ++i) {
    string comment;
    if (comment_mode_ == "read") {
      comment = result.reading;
    } else if (comment_mode_ == "static") {
      comment = "w32";
    }

    string preedit = result.composition.empty() ? input : result.composition;
    auto cand = New<SimpleCandidate>("win32_proxy", segment.start, segment.end,
                                     result.items[i], comment, preedit);
    cand->set_quality(100.0 - static_cast<double>(i));
    translation->Append(cand);
  }

  return translation->size() > 0 ? translation : nullptr;
}

}  // namespace win32_proxy
}  // namespace rime
