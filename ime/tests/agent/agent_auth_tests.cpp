#include "owo/agent/agent_auth.h"
#include "owo/agent/agent_pipe.h"
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iterator>
int wmain(int argc, wchar_t **argv) {
    if (argc == 3 || argc == 4) {
        std::string payload = "synthetic";
        if (argc == 4) {
            std::ifstream input(std::filesystem::path(argv[3]), std::ios::binary);
            if (!input)
                return 12;
            payload.assign(std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>());
        }
        const auto request =
            owo::agent::authenticate_agent_request(payload, std::filesystem::path(argv[1]));
        if (!request.ok)
            return 10;
        const auto result =
            owo::agent::exchange_agent_pipe(argv[2], request.payload, std::chrono::seconds(10));
        return result.ok && result.response == "synthetic response" ? 0 : 11;
    }
    if (owo::agent::authenticate_agent_request("", L"relative").ok)
        return 1;
    if (owo::agent::authenticate_agent_request("synthetic", L"relative").ok)
        return 2;
    return 0;
}
