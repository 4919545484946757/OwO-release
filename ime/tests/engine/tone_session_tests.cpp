#include "owo/engine/tone_session.h"
#include <winrt/base.h>
#include <iostream>
int main() {
    winrt::init_apartment();
    using namespace owo::engine;
    int failures = 0;
    const auto expect = [&](bool value, const char* label) {
        if (!value) { std::cerr << label << '\n'; ++failures; }
    };
    owo::config::AppConfig config;
    expect(!tone_session_allowed(config, "WeChat.exe", ToneDraftMode::reply), "default denial");
    config.corpus_learning_enabled = config.tone_reply_enabled = true;
    expect(tone_session_allowed(config, "WeChat.exe", ToneDraftMode::reply), "authorized reply");
    expect(!tone_session_allowed(config, "WeChat.exe", ToneDraftMode::continuation), "continuation needs own consent");
    expect(!tone_session_allowed(config, "KeePass.exe", ToneDraftMode::reply), "excluded application");
    expect(!tone_session_allowed(config, "", ToneDraftMode::reply), "unknown application");
    config.corpus_learning_paused = true;
    expect(!tone_session_allowed(config, "WeChat.exe", ToneDraftMode::reply), "pause denial");
    config.corpus_learning_paused = false; config.style_scene_mask = 6;
    expect(!tone_session_allowed(config, "WeChat.exe", ToneDraftMode::reply), "scene denial");
    config.style_scene_mask = 7; config.corpus_learning_allowed_apps = {"WINWORD.EXE"};
    expect(!tone_session_allowed(config, "WeChat.exe", ToneDraftMode::reply), "allowlist denial");
    expect(tone_local_endpoint(L"http://127.0.0.1:8080/v1"), "literal loopback");
    expect(tone_local_endpoint(L"https://[::1]:8080/v1"), "IPv6 loopback");
    for (const auto* url : {L"http://example.com/v1", L"https://127.0.0.1.example.com/v1",
        L"http://localhost/v1", L"https://user:secret@127.0.0.1/v1", L"http://127.0.0.1/v1?next=cloud",
        L"http://127.0.0.1/v1#cloud", L"file:///127.0.0.1/v1"})
        expect(!tone_local_endpoint(url), "unsafe endpoint did not become local");
    expect(tone_response_content(R"({"choices":[{"finish_reason":"stop","message":{"content":"好的\n好呀"}}]})") == "好的\n好呀", "normal content");
    expect(tone_response_content(R"({"choices":[{"finish_reason":"length","message":{"content":"好的"}}]})").empty(), "truncated content");
    expect(tone_response_content(R"({"choices":[{"finish_reason":"stop","message":{"content":"好的","tool_calls":[]}}]})").empty(), "tool call not accepted");
    expect(tone_response_content(R"({"choices":[]})").empty(), "missing choice");
    expect(tone_response_content("not json").empty(), "invalid JSON");
    expect(tone_response_content(std::string(65537, 'x')).empty(), "response bounded");
    const auto request = tone_model_request("test-model", "引号\"与换行\n");
    expect(request.find("tools") == request.npos && request.find("test-model") != request.npos, "no tools in request");
    expect(tone_model_request("", "message").empty(), "explicit model required");
    expect(tone_model_request("model", std::string(16385, 'x')).empty(), "prompt bounded");
    winrt::uninit_apartment();
    return failures ? 1 : 0;
}
