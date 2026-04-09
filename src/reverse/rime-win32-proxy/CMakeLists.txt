cmake_minimum_required(VERSION 3.12)
project(rime-win32-proxy LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

set(RIME_PLUGINS_DIR "rime-plugins" CACHE STRING "Plugin install dir under lib")

find_package(PkgConfig REQUIRED)
pkg_check_modules(RIME REQUIRED IMPORTED_TARGET rime)

add_library(rime-win32-proxy SHARED
  src/win32_proxy_module.cc
  src/win32_proxy_translator.cc
)

target_include_directories(rime-win32-proxy PRIVATE
  ${CMAKE_CURRENT_SOURCE_DIR}/src
)

target_link_libraries(rime-win32-proxy PRIVATE PkgConfig::RIME)

set_target_properties(rime-win32-proxy PROPERTIES
  OUTPUT_NAME "rime-win32-proxy"
  POSITION_INDEPENDENT_CODE ON
)

include(GNUInstallDirs)
install(TARGETS rime-win32-proxy
  LIBRARY DESTINATION ${CMAKE_INSTALL_LIBDIR}/${RIME_PLUGINS_DIR}
  RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}/${RIME_PLUGINS_DIR}
)
