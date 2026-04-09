#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)
CONTRACT_DIR=$(cd "${SCRIPT_DIR}/.." && pwd)
SRC_DIR=$(cd "${CONTRACT_DIR}/.." && pwd)

CANONICAL_PROTO="${CONTRACT_DIR}/proto/ime_proxy.proto"
HOST_BUILD_RS="${SRC_DIR}/ime-grpc-host/build.rs"
PROXY_CMAKE="${SRC_DIR}/rime-grpc-proxy/CMakeLists.txt"

fail() {
  echo "[FAIL] $*"
  exit 1
}

pass() {
  echo "[PASS] $*"
}

if [[ ! -f "${CANONICAL_PROTO}" ]]; then
  fail "canonical proto missing: ${CANONICAL_PROTO}"
fi

mapfile -t all_proto_files < <(
  find "${SRC_DIR}" \
    -type f \
    -name '*.proto' \
    -not -path '*/.cache/*' \
    -not -path '*/build/*' \
    -not -path '*/target/*' \
    | sort
)

if [[ ${#all_proto_files[@]} -eq 0 ]]; then
  fail "no .proto files found under ${SRC_DIR}"
fi

if [[ ${#all_proto_files[@]} -ne 1 ]]; then
  echo "[INFO] detected proto files:"
  printf '  - %s\n' "${all_proto_files[@]}"
  fail "expected exactly 1 proto file under ${SRC_DIR}"
fi

if [[ "${all_proto_files[0]}" != "${CANONICAL_PROTO}" ]]; then
  fail "single proto is not canonical path: ${all_proto_files[0]}"
fi

mapfile -t duplicated_name < <(
  find "${SRC_DIR}" \
    -type f \
    -name 'ime_proxy.proto' \
    -not -path '*/.cache/*' \
    -not -path '*/build/*' \
    -not -path '*/target/*' \
    | sort
)

if [[ ${#duplicated_name[@]} -ne 1 ]]; then
  echo "[INFO] ime_proxy.proto copies:"
  printf '  - %s\n' "${duplicated_name[@]}"
  fail "ime_proxy.proto must exist only once"
fi

if [[ ! -f "${HOST_BUILD_RS}" ]]; then
  fail "host build script missing: ${HOST_BUILD_RS}"
fi

if [[ ! -f "${PROXY_CMAKE}" ]]; then
  fail "proxy CMake file missing: ${PROXY_CMAKE}"
fi

if ! grep -Fq '../grpc-contract/proto/ime_proxy.proto' "${HOST_BUILD_RS}"; then
  fail "host build.rs is not wired to shared proto path"
fi

if ! grep -Fq '../grpc-contract/proto' "${HOST_BUILD_RS}"; then
  fail "host build.rs include path is not shared proto root"
fi

if ! grep -Fq '../grpc-contract/proto' "${PROXY_CMAKE}"; then
  fail "proxy CMakeLists does not reference shared proto root"
fi

if ! grep -Fq 'ime_proxy.proto' "${PROXY_CMAKE}"; then
  fail "proxy CMakeLists does not reference ime_proxy.proto"
fi

proto_hash=$(sha256sum "${CANONICAL_PROTO}" | awk '{print $1}')
pass "single proto source is enforced"
echo "[SUMMARY] canonical_proto=${CANONICAL_PROTO}"
echo "[SUMMARY] proto_sha256=${proto_hash}"
echo "[SUMMARY] host_build_rs=${HOST_BUILD_RS}"
echo "[SUMMARY] proxy_cmake=${PROXY_CMAKE}"
