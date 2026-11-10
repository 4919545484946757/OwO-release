#include "owo/model/libime_bridge.h"
#include <Windows.h>
#include <algorithm>
#include <array>
#include <chrono>
#include <cmath>
#include <cstring>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <vector>

namespace {
struct Bridge {
    HMODULE module{}; owo_libime_handle handle{};
    using Open = owo_libime_handle(*)(const char*, char*, size_t);
    using Batch = int(*)(owo_libime_handle, const char*, const char* const*, size_t, float*, char*, size_t);
    using Single = int(*)(owo_libime_handle, const char*, const char*, float*, char*, size_t);
    using Close = void(*)(owo_libime_handle);
    Batch batch{}; Single single{}; Close close{};
    bool load(const wchar_t* file, const std::string& model) {
        const auto path = std::filesystem::absolute(file);
        module = LoadLibraryExW(path.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
        if (!module) return false;
        auto open = reinterpret_cast<Open>(GetProcAddress(module, "owo_libime_open"));
        batch = reinterpret_cast<Batch>(GetProcAddress(module, "owo_libime_score_batch"));
        single = reinterpret_cast<Single>(GetProcAddress(module, "owo_libime_score"));
        close = reinterpret_cast<Close>(GetProcAddress(module, "owo_libime_close"));
        std::array<char, 512> error{};
        if (!open || !batch || !single || !close) return false;
        handle = open(model.c_str(), error.data(), error.size());
        if (!handle) std::cerr << error.data() << '\n';
        return handle != nullptr;
    }
    ~Bridge() { if (handle) close(handle); if (module) FreeLibrary(module); }
};
}
int wmain(int argc, wchar_t** argv) {
    if (argc != 5) return 2;
    // Test assets should use an ASCII path, matching libime's narrow model ABI.
    const auto bytes = std::filesystem::path(argv[3]).u8string();
    const std::string model(reinterpret_cast<const char*>(bytes.data()), bytes.size());
    Bridge before, after;
    if (!before.load(argv[1], model) || !after.load(argv[2], model)) return 3;
    std::ofstream report(std::filesystem::path(argv[4]), std::ios::binary);
    if (!report) return 8;
    const std::vector<std::vector<std::string>> pools{
        {"工作", "世界", "问题", "计划", "方法", "中文", "你好", "结果", "首选", "可以", "功能", "输入", "候选", "生成", "速度", "模型"},
        {"价值大妈", "价值大吗", "联系大妈", "联系大吗", "购买芝麻", "购买芝吗"},
        {"列表为展开", "列表未展开", "列表位展开", "列表微展开", "窗口为打开", "窗口未打开"},
        {"以货物运输的速度", "一货物运输的速度", "以我的视角", "一我的视角", "修复类似问题", "朽腐类似问题"}
    };
    unsigned comparisons = 0;
    for (const auto& context : std::vector<std::string>{"", "解决", "请根据当前的输入内容继续生成合适的中文候选"}) {
        for (std::size_t p = 0; p < pools.size(); ++p) {
            std::vector<const char*> candidates;
            for (const auto& text : pools[p]) candidates.push_back(text.c_str());
            std::vector<float> baseline(candidates.size()), optimized(candidates.size());
            std::array<char, 512> error{};
            if (!before.batch(before.handle, context.c_str(), candidates.data(), candidates.size(), baseline.data(), error.data(), error.size()) ||
                !after.batch(after.handle, context.c_str(), candidates.data(), candidates.size(), optimized.data(), error.data(), error.size())) return 4;
            for (std::size_t i = 0; i < baseline.size(); ++i) {
                if (!std::isfinite(baseline[i]) || std::memcmp(&baseline[i], &optimized[i], sizeof(float))) return 5;
                float a{}, b{};
                if (!before.single(before.handle, context.c_str(), candidates[i], &a, error.data(), error.size()) ||
                    !after.single(after.handle, context.c_str(), candidates[i], &b, error.data(), error.size()) ||
                    std::memcmp(&a, &b, sizeof(float))) return 6;
                comparisons += 2;
            }
            std::vector<double> old_times, new_times;
            for (unsigned i = 0; i < 110; ++i) {
                auto run = [&](Bridge& bridge, std::vector<double>& times) {
                    const auto start = std::chrono::steady_clock::now();
                    if (!bridge.batch(bridge.handle, context.c_str(), candidates.data(), candidates.size(), optimized.data(), error.data(), error.size())) return false;
                    if (i >= 10) times.push_back(std::chrono::duration<double, std::micro>(std::chrono::steady_clock::now()-start).count());
                    return true;
                };
                // Alternate measurement order to limit temperature/cache bias.
                if (i % 2 ? (!run(after, new_times) || !run(before, old_times)) : (!run(before, old_times) || !run(after, new_times))) return 7;
            }
            std::sort(old_times.begin(), old_times.end()); std::sort(new_times.begin(), new_times.end());
            report << "{\"context_bytes\":" << context.size() << ",\"pool\":" << p << ",\"candidates\":" << candidates.size()
                << ",\"samples\":100,\"before_p50_us\":" << old_times[49] << ",\"after_p50_us\":" << new_times[49]
                << ",\"before_p95_us\":" << old_times[94] << ",\"after_p95_us\":" << new_times[94] << "}\n";
        }
    }
    std::cout << "Bit-identical batch + scalar score comparisons: " << comparisons << "; 12 alternating timing cases completed.\n";
    return 0;
}
