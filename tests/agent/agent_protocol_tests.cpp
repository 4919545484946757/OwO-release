#include "owo/agent/agent_pipe.h"
#include "owo/agent/agent_protocol.h"

#include <string>

int main() {
    const std::string session = "0123456789abcdef0123456789abcdef";
    owo::agent::Request request;
    request.action = owo::agent::Action::submit;
    request.session_id = session;
    request.user_input = "bangwozhaowenjian";
    request.application = {42, "notepad.exe", "Notepad", false};
    request.session_context = "必要上下文";
    const auto encoded_request = owo::agent::encode_request(request);
    const auto decoded_request = owo::agent::decode_request(encoded_request);
    if (!decoded_request.ok || decoded_request.value != request) return 1;
    if (owo::agent::decode_request(encoded_request.substr(0, encoded_request.size() - 1)).ok)
        return 2;
    auto unknown = encoded_request;
    unknown.insert(unknown.size() - 1, ",\"unknown\":false");
    if (owo::agent::decode_request(unknown).ok) return 3;

    owo::agent::Response response;
    response.session_id = session;
    response.message = "请选择操作";
    response.commands = {{"open-folder", "打开文件夹", false},
                         {"delete-file", "删除文件", true}};
    response.status = owo::agent::Status::agent_mode;
    response.page = 2;
    response.has_more = true;
    const auto encoded_response = owo::agent::encode_response(response);
    const auto decoded_response = owo::agent::decode_response(encoded_response);
    if (!decoded_response.ok || decoded_response.value != response) return 4;
    if (owo::agent::decode_response("{}").ok) return 5;
    response.commands.push_back({"open-folder", "重复", false});
    if (owo::agent::decode_response(owo::agent::encode_response(response)).ok) return 12;
    response.commands.pop_back();
    auto escaped_response = encoded_response;
    const auto message_begin = escaped_response.find("请选择操作");
    if (message_begin == std::string::npos) return 9;
    escaped_response.replace(message_begin, std::string("请选择操作").size(),
                             "\\u8bf7\\u9009\\u62e9\\u64cd\\u4f5c");
    const auto decoded_escaped = owo::agent::decode_response(escaped_response);
    if (!decoded_escaped.ok || decoded_escaped.value.message != "请选择操作") return 10;
    auto invalid_surrogate = escaped_response;
    const auto unicode = invalid_surrogate.find("\\u8bf7");
    invalid_surrogate.replace(unicode, 6, "\\ud800");
    if (owo::agent::decode_response(invalid_surrogate).ok) return 11;

    const auto filtered = owo::agent::filter_sensitive_text(
        "password=hunter2 token:abc123 card=4111111111111111", 4096);
    if (!filtered.ok || !filtered.redacted ||
        filtered.value.find("hunter2") != std::string::npos ||
        filtered.value.find("abc123") != std::string::npos ||
        filtered.value.find("4111111111111111") != std::string::npos) return 6;
    if (owo::agent::filter_sensitive_text(std::string("\xff", 1)).ok) return 7;
    if (!owo::agent::valid_agent_pipe_name(LR"(\\.\pipe\OwO.Agent.External.v1)") ||
        owo::agent::valid_agent_pipe_name(LR"(\\server\pipe\agent)") ||
        owo::agent::valid_agent_pipe_name(LR"(\\.\pipe\..\agent)")) return 8;
    return 0;
}
