#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <Windows.h>

#include <chrono>
#include <iostream>
#include <string>
#include <string_view>

namespace {
std::string utf8(const std::wstring_view value) {
    if (value.empty()) return {};
    const int size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS,
        value.data(), static_cast<int>(value.size()), nullptr, 0, nullptr, nullptr);
    if (size <= 0) return {};
    std::string result(static_cast<std::size_t>(size), '\0');
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, value.data(),
            static_cast<int>(value.size()), result.data(), size, nullptr, nullptr) != size)
        return {};
    return result;
}
}  // namespace

int wmain(int argc, wchar_t** argv) {
    const bool shutdown = argc > 1 && std::wstring_view(argv[1]) == L"--shutdown";
    const bool update = argc > 1 && std::wstring_view(argv[1]) == L"--update";
    const bool learn = argc > 1 && std::wstring_view(argv[1]) == L"--learn";
    if (learn && (argc < 4 || argc > 5)) {
        std::cerr << "usage: owo_ipc_shell --learn <pinyin> <text> [context]\n";
        return 1;
    }
    const std::string input = learn ? utf8(argv[2])
        : argc > 1 && !shutdown && !update ? utf8(argv[1]) : "test";
    owo::protocol::Message request{
        shutdown ? owo::protocol::MessageType::shutdown_request
                 : update ? owo::protocol::MessageType::candidate_update_request
                          : learn ? owo::protocol::MessageType::candidate_committed
                          : owo::protocol::MessageType::candidate_request,
        1, 1, learn ? utf8(argv[3]) : input};
    if (learn) {
        request.input = input;
        if (argc == 5) request.context = utf8(argv[4]);
    } else if (!shutdown && !update && argc > 2) {
        if (std::wstring_view(argv[2]) == L"--expanded")
            request.expanded = true;
        else
            request.page = static_cast<std::uint64_t>(std::stoull(std::wstring(argv[2])));
    }
    const auto result = owo::ipc::exchange(
        owo::ipc::kCorePipeName, owo::protocol::encode_message(request),
        std::chrono::milliseconds(2000));
    if (!result.status) {
        std::cerr << result.status.message << '\n';
        return 2;
    }
    const auto decoded = owo::protocol::decode_message(result.response);
    const auto expected_type = shutdown || learn
        ? owo::protocol::MessageType::acknowledgement
        : update ? owo::protocol::MessageType::candidate_update_response
                 : owo::protocol::MessageType::candidate_response;
    if (!decoded.validation || decoded.message.type != expected_type ||
        decoded.message.request_id != request.request_id ||
        decoded.message.context_generation != request.context_generation) {
        std::cerr << "invalid or stale response\n";
        return 3;
    }
    if (shutdown || learn) {
        std::cout << decoded.message.text << '\n';
    } else {
        if (!decoded.message.syllables.empty()) {
            for (std::size_t index = 0; index < decoded.message.syllables.size(); ++index) {
                if (index != 0) std::cout << '\'';
                std::cout << decoded.message.syllables[index];
            }
            std::cout << '\n';
        }
        for (std::size_t index = 0; index < decoded.message.candidates.size(); ++index) {
            std::cout << index + 1 << ". " << decoded.message.candidates[index];
            if (!decoded.message.candidate_groups.empty())
                std::cout << " [group=" << decoded.message.candidate_groups[index] << ']';
            std::cout << '\n';
        }
        std::cout << "page=" << decoded.message.page
                  << " has_more=" << decoded.message.has_more << '\n';
        if (decoded.message.model_pending) std::cout << "model_pending\n";
    }
    return 0;
}
