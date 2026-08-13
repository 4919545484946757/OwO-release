#include "owo/config/config_monitor.h"

#include <chrono>
#include <filesystem>
#include <fstream>
#include <stop_token>
#include <thread>

using namespace std::chrono_literals;

int main(const int argc, char** argv) {
    if (argc != 2) return 1;
    const std::filesystem::path root(argv[1]);
    std::error_code error;
    std::filesystem::remove_all(root, error);
    std::filesystem::create_directories(root, error);
    if (error) return 2;
    const auto path = root / "owo.conf";

    owo::config::ConfigMonitor monitor;
    const auto started = monitor.start(path, 20ms);
    if (!started.success || !started.used_defaults || monitor.generation() != 1 ||
        *monitor.snapshot() != owo::config::AppConfig{}) return 3;
    if (monitor.start(path, 20ms).success) return 4;

    owo::config::ConfigStore writer;
    if (!writer.load(path).success) return 5;
    auto first = writer.snapshot();
    first.candidate_page_size = 9;
    if (!writer.save(first).success || !monitor.wait_for_generation(1, 2s)) return 6;
    const auto first_snapshot = monitor.snapshot();
    if (monitor.generation() != 2 || first_snapshot->candidate_page_size != 9) return 7;

    { std::ofstream output(path, std::ios::binary | std::ios::trunc); output << "broken"; }
    std::this_thread::sleep_for(100ms);
    if (monitor.generation() != 2 || monitor.snapshot()->candidate_page_size != 9 ||
        monitor.last_diagnostic().empty()) return 8;

    auto second = first;
    second.model_timeout_ms = 90;
    if (!writer.save(second).success || !monitor.wait_for_generation(2, 2s)) return 9;
    if (monitor.generation() != 3 || monitor.snapshot()->model_timeout_ms != 90) return 10;

    std::stop_source cancelled;
    cancelled.request_stop();
    if (monitor.wait_for_generation(3, 1s, cancelled.get_token())) return 11;
    monitor.stop();
    const auto stopped_generation = monitor.generation();
    second.model_timeout_ms = 100;
    if (!writer.save(second).success) return 12;
    std::this_thread::sleep_for(80ms);
    if (monitor.generation() != stopped_generation) return 13;

    std::filesystem::remove_all(root, error);
    return 0;
}
