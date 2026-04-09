#include <algorithm>
#include <cctype>
#include <cstdlib>
#include <iostream>
#include <string>

#include <rime/key_event.h>

#include "grpc_client.h"

namespace {

int ParsePositiveInt(const char* raw, int fallback) {
  if (!raw || !*raw) {
    return fallback;
  }

  const int parsed = std::atoi(raw);
  return parsed > 0 ? parsed : fallback;
}

bool ParseBoolEnv(const char* name, bool fallback) {
  const char* raw = std::getenv(name);
  if (!raw || !*raw) {
    return fallback;
  }

  std::string value(raw);
  std::transform(value.begin(), value.end(), value.begin(), [](unsigned char c) {
    return static_cast<char>(std::tolower(c));
  });

  if (value == "1" || value == "true" || value == "yes" || value == "on") {
    return true;
  }
  if (value == "0" || value == "false" || value == "no" || value == "off") {
    return false;
  }

  return fallback;
}

std::string ReadEnvString(const char* name, const std::string& fallback) {
  const char* raw = std::getenv(name);
  if (!raw || !*raw) {
    return fallback;
  }
  return raw;
}

std::string NormalizeInput(std::string input) {
  std::transform(input.begin(), input.end(), input.begin(), [](unsigned char c) {
    return static_cast<char>(std::tolower(c));
  });
  return input;
}

std::string CandidatePreview(const rime::grpc_proxy::QueryResult& result,
                             size_t limit) {
  if (result.candidates.empty() || limit == 0) {
    return "<none>";
  }

  const size_t count = std::min(result.candidates.size(), limit);
  std::string out;
  for (size_t i = 0; i < count; ++i) {
    if (i > 0) {
      out.append(" | ");
    }
    out.append(result.candidates[i].text);
  }
  if (result.candidates.size() > count) {
    out.append(" | ...");
  }
  return out;
}

}  // namespace

int main() {
  rime::grpc_proxy::GrpcClientConfig cfg;
  cfg.host = ReadEnvString("IME_GRPC_HOST", cfg.host);
  cfg.port = ParsePositiveInt(std::getenv("IME_GRPC_PORT"), cfg.port);
  cfg.timeout_ms =
      ParsePositiveInt(std::getenv("IME_GRPC_TIMEOUT_MS"), cfg.timeout_ms);
  cfg.max_candidates = ParsePositiveInt(std::getenv("IME_GRPC_MAX_CANDIDATES"),
                                        cfg.max_candidates);
    cfg.debug_stop_mode =
      ParseBoolEnv("IME_GRPC_DEBUG_STOP_MODE", cfg.debug_stop_mode);
  cfg.frontend_id = ReadEnvString("IME_GRPC_FRONTEND_ID", cfg.frontend_id);
  cfg.schema_id = ReadEnvString("IME_GRPC_SCHEMA_ID", cfg.schema_id);

  std::string input =
      NormalizeInput(ReadEnvString("IME_REPLAY_INPUT", "nihao"));

  if (input.empty()) {
    std::cerr << "IME_REPLAY_INPUT is empty" << std::endl;
    return 2;
  }

  rime::grpc_proxy::GrpcImeClient client(cfg);

  std::cout << "replay_start endpoint=" << cfg.host << ":" << cfg.port
            << " input='" << input << "' max_candidates=" << cfg.max_candidates
            << std::endl;

  std::string prefix;
  for (size_t i = 0; i < input.size(); ++i) {
    const char ch = input[i];
    if (!(std::isalnum(static_cast<unsigned char>(ch)) || ch == '\'')) {
      continue;
    }

    prefix.push_back(ch);

    const int keycode = static_cast<unsigned char>(ch);
    const rime::KeyEvent key_event(keycode, 0);

    if (!client.SendKeyEvent(key_event)) {
      std::cerr << "step " << (i + 1) << " send_key_event_failed input='"
                << prefix << "'" << std::endl;
      return 1;
    }

    rime::grpc_proxy::QueryResult result;
    if (!client.QueryCandidates(prefix, cfg.max_candidates, &result)) {
      std::cerr << "step " << (i + 1) << " query_candidates_failed input='"
                << prefix << "'" << std::endl;
      return 1;
    }

    std::cout << "step " << (i + 1) << " input='" << prefix << "'"
              << " composition='" << result.composition << "'"
              << " reading='" << result.reading << "'"
              << " cand_n=" << result.candidates.size()
              << " top=[" << CandidatePreview(result, 5) << "]"
              << std::endl;
  }

  return 0;
}
