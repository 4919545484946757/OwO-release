cmake_minimum_required(VERSION 3.24)
project(OwOAgentIpcExample LANGUAGES CXX)

if(NOT WIN32)
    message(FATAL_ERROR "OwO Agent IPC example currently targets Windows.")
endif()

add_library(owo_agent_protocol STATIC
    src/agent_protocol.cpp
    src/agent_pipe.cpp)
target_include_directories(owo_agent_protocol PUBLIC include)
target_compile_features(owo_agent_protocol PUBLIC cxx_std_20)
target_compile_definitions(owo_agent_protocol PRIVATE WIN32_LEAN_AND_MEAN NOMINMAX)

add_executable(owo_agent_mock examples/mock-agent/main.cpp)
target_link_libraries(owo_agent_mock PRIVATE owo_agent_protocol advapi32)
target_compile_features(owo_agent_mock PRIVATE cxx_std_20)
target_compile_definitions(owo_agent_mock PRIVATE WIN32_LEAN_AND_MEAN NOMINMAX)

if(MSVC)
    target_compile_options(owo_agent_protocol PRIVATE /W4 /permissive- /utf-8 /EHsc)
    target_compile_options(owo_agent_mock PRIVATE /W4 /permissive- /utf-8 /EHsc)
    set_property(TARGET owo_agent_protocol owo_agent_mock PROPERTY
                 MSVC_RUNTIME_LIBRARY "MultiThreaded$<$<CONFIG:Debug>:Debug>")
endif()
