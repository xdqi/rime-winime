#include "grpc_component_config.h"

#include <rime/config.h>
#include <rime/engine.h>
#include <rime/schema.h>

namespace rime {
namespace grpc_proxy {

namespace {

static std::string ResolveBaseNamespace(const Ticket& ticket,
                                        const std::string& fallback_namespace) {
  if (!ticket.name_space.empty()) {
    return ticket.name_space;
  }
  return fallback_namespace;
}

static void LoadStringWithFallback(Config* config,
                                   const std::string& base_ns,
                                   const std::string& fallback_ns,
                                   const std::string& key,
                                   std::string* out) {
  if (!config || !out) {
    return;
  }
  const std::string primary = base_ns + "/" + key;
  if (config->GetString(primary, out)) {
    return;
  }
  const std::string secondary = fallback_ns + "/" + key;
  (void)config->GetString(secondary, out);
}

static void LoadIntWithFallback(Config* config,
                                const std::string& base_ns,
                                const std::string& fallback_ns,
                                const std::string& key,
                                int* out) {
  if (!config || !out) {
    return;
  }
  const std::string primary = base_ns + "/" + key;
  if (config->GetInt(primary, out)) {
    return;
  }
  const std::string secondary = fallback_ns + "/" + key;
  (void)config->GetInt(secondary, out);
}

static void LoadBoolWithFallback(Config* config,
                                 const std::string& base_ns,
                                 const std::string& fallback_ns,
                                 const std::string& key,
                                 bool* out) {
  if (!config || !out) {
    return;
  }
  const std::string primary = base_ns + "/" + key;
  if (config->GetBool(primary, out)) {
    return;
  }
  const std::string secondary = fallback_ns + "/" + key;
  (void)config->GetBool(secondary, out);
}

}  // namespace

GrpcClientConfig LoadGrpcClientConfig(const Ticket& ticket,
                                      const std::string& fallback_namespace) {
  GrpcClientConfig cfg;

  if (!ticket.engine || !ticket.engine->schema()) {
    return cfg;
  }

  cfg.schema_id = ticket.engine->schema()->schema_id();

  Config* config = ticket.engine->schema()->config();
  const std::string base_ns = ResolveBaseNamespace(ticket, fallback_namespace);

  LoadStringWithFallback(config, base_ns, fallback_namespace, "host", &cfg.host);
  LoadIntWithFallback(config, base_ns, fallback_namespace, "port", &cfg.port);
  LoadIntWithFallback(config, base_ns, fallback_namespace, "timeout_ms",
                      &cfg.timeout_ms);
  LoadIntWithFallback(config, base_ns, fallback_namespace, "max_candidates",
                      &cfg.max_candidates);
  LoadBoolWithFallback(config, base_ns, fallback_namespace, "debug_stop_mode",
                       &cfg.debug_stop_mode);
  LoadStringWithFallback(config, base_ns, fallback_namespace, "frontend_id",
                         &cfg.frontend_id);

  if (cfg.port <= 0) {
    cfg.port = 50051;
  }
  if (cfg.timeout_ms <= 0) {
    cfg.timeout_ms = 120;
  }
  if (cfg.max_candidates <= 0) {
    cfg.max_candidates = 9;
  }

  return cfg;
}

}  // namespace grpc_proxy
}  // namespace rime
