cmake_minimum_required(VERSION 3.16)
project(rime-remote LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

if(NOT TARGET gRPC::grpc++)
  find_package(Protobuf CONFIG REQUIRED)
  find_package(gRPC CONFIG REQUIRED)
  # Promote imported targets to global visibility so librime/src can see them.
  set_property(TARGET gRPC::grpc++ PROPERTY IMPORTED_GLOBAL TRUE)
  set_property(TARGET protobuf::libprotobuf PROPERTY IMPORTED_GLOBAL TRUE)
endif()

# Locate proto file — try sibling grpc-contract-v2 directory first,
# then fall back to the deps/sogou location.
set(PROTO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../grpc-contract-v2/proto")
if(NOT EXISTS "${PROTO_ROOT}/rime_service.proto")
  set(PROTO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../../../deps/sogou/src/grpc-contract-v2/proto")
endif()
set(PROTO_FILE "${PROTO_ROOT}/rime_service.proto")
set(GEN_DIR "${CMAKE_CURRENT_BINARY_DIR}/generated")

file(MAKE_DIRECTORY "${GEN_DIR}")

add_custom_command(
  OUTPUT
    "${GEN_DIR}/rime_service.pb.cc"
    "${GEN_DIR}/rime_service.pb.h"
    "${GEN_DIR}/rime_service.grpc.pb.cc"
    "${GEN_DIR}/rime_service.grpc.pb.h"
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

# Reuse grpc_client from rime-grpc-proxy-v2.
set(GRPC_CLIENT_DIR "${CMAKE_CURRENT_SOURCE_DIR}/../rime-grpc-proxy-v2/src")

set(plugin_src
  src/remote_module.cc
  src/remote_processor.cc
  src/remote_segmentor.cc
  src/remote_translator.cc
  "${GRPC_CLIENT_DIR}/grpc_client.cc"
  "${GEN_DIR}/rime_service.pb.cc"
  "${GEN_DIR}/rime_service.grpc.pb.cc"
)

if(DEFINED rime_library)
  # Built as part of librime.
  add_library(rime-remote-objs OBJECT ${plugin_src})
  target_include_directories(rime-remote-objs PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${GRPC_CLIENT_DIR}
    ${GEN_DIR}
  )
  target_link_libraries(rime-remote-objs PRIVATE
    gRPC::grpc++ protobuf::libprotobuf
    mpr wsock32 ws2_32 crypt32 bcrypt
  )

  if(BUILD_SHARED_LIBS)
    set_target_properties(rime-remote-objs PROPERTIES POSITION_INDEPENDENT_CODE ON)
  endif()

  set(plugin_name rime-remote PARENT_SCOPE)
  set(plugin_objs $<TARGET_OBJECTS:rime-remote-objs> PARENT_SCOPE)
  set(plugin_deps ${rime_library} gRPC::grpc++ protobuf::libprotobuf PARENT_SCOPE)
  set(plugin_modules "remote" PARENT_SCOPE)
else()
  # Standalone build (not integrated into librime).
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(RIME REQUIRED IMPORTED_TARGET rime)

  add_library(rime-remote SHARED ${plugin_src})
  target_include_directories(rime-remote PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${GRPC_CLIENT_DIR}
    ${GEN_DIR}
  )
  target_link_libraries(rime-remote PRIVATE
    PkgConfig::RIME
    gRPC::grpc++ protobuf::libprotobuf
  )
endif()
