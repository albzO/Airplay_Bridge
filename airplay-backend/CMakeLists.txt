cmake_minimum_required(VERSION 3.20)
project(airplay_backend LANGUAGES C CXX)
if(NOT WIN32 OR NOT MINGW)
  message(FATAL_ERROR "Use the Windows MSYS2 UCRT64 GCC toolchain")
endif()
set(CMAKE_C_STANDARD 17)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
find_package(Python3 REQUIRED COMPONENTS Interpreter)
find_package(OpenSSL REQUIRED)
set(UPSTREAM "${CMAKE_CURRENT_SOURCE_DIR}/../upstream/airplay-cli")
set(GENERATED "${CMAKE_CURRENT_BINARY_DIR}/generated")
set_property(DIRECTORY APPEND PROPERTY CMAKE_CONFIGURE_DEPENDS
  "${CMAKE_CURRENT_SOURCE_DIR}/select_upstream.py"
  "${CMAKE_CURRENT_SOURCE_DIR}/auth_auto.inc"
  "${CMAKE_CURRENT_SOURCE_DIR}/probe_entry.inc"
  "${CMAKE_CURRENT_SOURCE_DIR}/audio_entry.inc"
  "${CMAKE_CURRENT_SOURCE_DIR}/volume_entry.inc"
  "${CMAKE_CURRENT_SOURCE_DIR}/events_entry.inc"
  "${CMAKE_CURRENT_SOURCE_DIR}/windows_io.inc")
execute_process(COMMAND "${Python3_EXECUTABLE}" "${CMAKE_CURRENT_SOURCE_DIR}/select_upstream.py"
  "${UPSTREAM}" "${GENERATED}" COMMAND_ERROR_IS_FATAL ANY)
add_executable(airplay-backend main.c windows_port.c logging.c shm_disabled.c raw_codec.c pcm_source.c audio_nonce.c
  "${GENERATED}/raw_alac.c"
  "${GENERATED}/probe_client.c" "${GENERATED}/ap2_hap.c" "${GENERATED}/ap2_io.c"
  "${GENERATED}/ap2_ptp.c" "${GENERATED}/ap2_plist.c"
  "${GENERATED}/ap2_bplist.cpp" "${GENERATED}/bplist.cpp")
target_include_directories(airplay-backend PRIVATE "${GENERATED}" "${CMAKE_CURRENT_SOURCE_DIR}"
  "${UPSTREAM}/libraop/crosstools/src")
target_compile_definitions(airplay-backend PRIVATE _GNU_SOURCE OPENSSL_SUPPRESS_DEPRECATED
  WIN32_LEAN_AND_MEAN NOMINMAX _WIN32_WINNT=0x0602)
target_compile_options(airplay-backend PRIVATE -Wall -Wextra -ffunction-sections -fdata-sections)
target_link_options(airplay-backend PRIVATE -municode -Wl,--gc-sections)
target_link_libraries(airplay-backend PRIVATE OpenSSL::Crypto winpthread ws2_32 iphlpapi)

