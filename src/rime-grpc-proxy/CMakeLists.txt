cmake_minimum_required(VERSION 3.16)
project(rime-grpc-proxy LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

set(RIME_PLUGINS_DIR "rime-plugins" CACHE STRING "Plugin install dir under lib")

find_package(PkgConfig REQUIRED)
pkg_check_modules(RIME REQUIRED IMPORTED_TARGET rime)

find_package(Protobuf REQUIRED)
find_package(gRPC CONFIG REQUIRED)

set(PROTO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../grpc-contract/proto")
set(PROTO_FILE "${PROTO_ROOT}/ime_proxy.proto")
set(GEN_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated")

file(MAKE_DIRECTORY "${GEN_DIR}")

add_custom_command(
  OUTPUT
    "${GEN_DIR}/ime_proxy.pb.cc"
    "${GEN_DIR}/ime_proxy.pb.h"
    "${GEN_DIR}/ime_proxy.grpc.pb.cc"
    "${GEN_DIR}/ime_proxy.grpc.pb.h"
  COMMAND protobuf::protoc
  ARGS
    --proto_path=${PROTO_ROOT}
    --cpp_out=${GEN_DIR}
    --grpc_out=${GEN_DIR}
    --plugin=protoc-gen-grpc=$<TARGET_FILE:gRPC::grpc_cpp_plugin>
    ${PROTO_FILE}
  DEPENDS ${PROTO_FILE}
  VERBATIM
)

add_library(rime-grpc-proxy SHARED
  src/grpc_proxy_module.cc
  src/grpc_component_config.cc
  src/grpc_client.cc
  src/grpc_key_event_processor.cc
  src/grpc_proxy_translator.cc
  src/grpc_commit_observer.cc
  "${GEN_DIR}/ime_proxy.pb.cc"
  "${GEN_DIR}/ime_proxy.grpc.pb.cc"
)

add_executable(grpc-replay
  src/grpc_replay_main.cc
)

target_include_directories(rime-grpc-proxy PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/src
  ${GEN_DIR}
)

target_include_directories(grpc-replay PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/src
  ${GEN_DIR}
)

target_link_libraries(rime-grpc-proxy PRIVATE
  PkgConfig::RIME
  gRPC::grpc++
  protobuf::libprotobuf
)

target_link_libraries(grpc-replay PRIVATE
  rime-grpc-proxy
)

set_target_properties(rime-grpc-proxy PROPERTIES
  OUTPUT_NAME "rime-grpc-proxy"
  POSITION_INDEPENDENT_CODE ON
)

include(GNUInstallDirs)
install(TARGETS rime-grpc-proxy
  LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}/${RIME_PLUGINS_DIR}
  RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}/${RIME_PLUGINS_DIR}
)
