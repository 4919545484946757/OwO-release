// 读取 contracts/agent-ipc/fixtures 黄金样例（与 Rust 网关测试共用）。
// 文件名约定：<request|response>.<名称>.<valid|invalid>.json，由 check_fixtures.py 与 manifest 对齐。
#include "owo/agent/agent_protocol.h"

#include <filesystem>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>

namespace fs = std::filesystem;

namespace {
bool ends_with(const std::string& value, const std::string& suffix) {
    return value.size() >= suffix.size() &&
           value.compare(value.size() - suffix.size(), suffix.size(), suffix) == 0;
}
}  // namespace

int main(int argc, char** argv) {
    if (argc != 2) return 100;
    const fs::path directory = fs::path(argv[1]);
    int checked = 0;
    int failures = 0;
    for (const auto& item : fs::directory_iterator(directory)) {
        const auto name = item.path().filename().string();
        if (!ends_with(name, ".json")) continue;
        std::ifstream input(item.path(), std::ios::binary);
        std::stringstream buffer;
        buffer << input.rdbuf();
        const auto text = buffer.str();
        const bool valid = ends_with(name, ".valid.json");
        const bool request = name.rfind("request.", 0) == 0;
        bool ok = false;
        bool roundtrip = true;
        std::string diagnostic;
        if (request) {
            const auto decoded = owo::agent::decode_request(text);
            ok = decoded.ok;
            diagnostic = decoded.diagnostic;
            if (ok && valid) {
                const auto again = owo::agent::decode_request(owo::agent::encode_request(decoded.value));
                roundtrip = again.ok && again.value == decoded.value;
            }
        } else {
            const auto decoded = owo::agent::decode_response(text);
            ok = decoded.ok;
            diagnostic = decoded.diagnostic;
            if (ok && valid) {
                const auto again = owo::agent::decode_response(owo::agent::encode_response(decoded.value));
                roundtrip = again.ok && again.value == decoded.value;
            }
        }
        ++checked;
        if (ok != valid || !roundtrip) {
            ++failures;
            std::cerr << "FAIL " << name << " ok=" << ok << " roundtrip=" << roundtrip << " "
                      << diagnostic << '\n';
        }
    }
    std::cout << "checked " << checked << " fixtures, " << failures << " failures\n";
    return checked == 0 ? 101 : (failures == 0 ? 0 : 1);
}
