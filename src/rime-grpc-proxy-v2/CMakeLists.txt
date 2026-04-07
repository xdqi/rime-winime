cmake_minimum_required(VERSION 3.16)
project(rime-grpc-proxy-v2 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

set(RIME_PLUGINS_DIR "rime-plugins" CACHE STRING "Plugin install dir under lib")

find_package(Protobuf CONFIG REQUIRED)
find_package(gRPC CONFIG REQUIRED)

# 提升导入的目标为全局可见，否则外部的 librime/src 作用域下找不到 gRPC::grpc++
set_property(TARGET gRPC::grpc++ PROPERTY IMPORTED_GLOBAL TRUE)
set_property(TARGET protobuf::libprotobuf PROPERTY IMPORTED_GLOBAL TRUE)

# 如果是外部独立构建，则寻找 RIME 依赖
if(NOT DEFINED rime_library)
  find_package(PkgConfig REQUIRED)
  pkg_check_modules(RIME REQUIRED IMPORTED_TARGET rime)
endif()

set(PROTO_ROOT "${CMAKE_CURRENT_SOURCE_DIR}/../grpc-contract-v2/proto")
if(NOT EXISTS "${PROTO_ROOT}/rime_service.proto")
  # 当代码被 mklink /j 或者复制到 librime/plugins 里时，原本的相对路径 ../ 会失效，
  # 直接指回原本 deps/sogou 存放 proto 的精确位置：
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

set(plugin_src
  src/grpc_proxy_module.cc
  src/grpc_client.cc
  src/grpc_key_event_processor.cc
  "${GEN_DIR}/rime_service.pb.cc"
  "${GEN_DIR}/rime_service.grpc.pb.cc"
)

# 判断是否是内置到 librime 中编译
if(DEFINED rime_library)
  # rime_library 是 Rime 构建环境里的 target 名称
  add_library(rime-grpc-proxy-v2-objs OBJECT ${plugin_src})
  target_include_directories(rime-grpc-proxy-v2-objs PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${GEN_DIR}
  )
  target_link_libraries(rime-grpc-proxy-v2-objs PRIVATE 
    gRPC::grpc++ protobuf::libprotobuf 
    mpr wsock32 ws2_32 crypt32 bcrypt
  )
  
  if(BUILD_SHARED_LIBS)
    set_target_properties(rime-grpc-proxy-v2-objs PROPERTIES POSITION_INDEPENDENT_CODE ON)
  endif()
  
  # 设置必须传递给 Rime 父作用域的变量，才能打包进 DLL
  set(plugin_name rime-grpc-proxy-v2 PARENT_SCOPE)
  set(plugin_objs $<TARGET_OBJECTS:rime-grpc-proxy-v2-objs> PARENT_SCOPE)
  set(plugin_deps ${rime_library} gRPC::grpc++ protobuf::libprotobuf PARENT_SCOPE)
  set(plugin_modules "grpc_proxy_v2" PARENT_SCOPE)
else()
  # 这是原有的外部扩展构建方式
  add_library(rime-grpc-proxy-v2 SHARED ${plugin_src})

  target_include_directories(rime-grpc-proxy-v2 PRIVATE
    ${CMAKE_CURRENT_SOURCE_DIR}/src
    ${GEN_DIR}
  )

  target_link_libraries(rime-grpc-proxy-v2 PRIVATE
    PkgConfig::RIME
    gRPC::grpc++
    protobuf::libprotobuf
  )

  set_target_properties(rime-grpc-proxy-v2 PROPERTIES
    OUTPUT_NAME "rime-grpc-proxy-v2"
    POSITION_INDEPENDENT_CODE ON
  )

  include(GNUInstallDirs)
  install(TARGETS rime-grpc-proxy-v2
    LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}/${RIME_PLUGINS_DIR}
    RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}/${RIME_PLUGINS_DIR}
  )
endif()
