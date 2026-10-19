#include "owo/config/config_monitor.h"

#include <system_error>

namespace owo::config {
namespace {

struct FileState {
    bool exists{};
    std::uintmax_t size{};
    std::filesystem::file_time_type write_time{};
    bool operator==(const FileState&) const = default;
};

FileState observe(const std::filesystem::path& path) {
    std::error_code error;
    const bool exists = std::filesystem::exists(path, error);
    if (error || !exists) return {};
    const auto size = std::filesystem::file_size(path, error);
    if (error) return {};
    const auto write_time = std::filesystem::last_write_time(path, error);
    if (error) return {};
    return {true, size, write_time};
}

}  // namespace

ConfigMonitor::ConfigMonitor() : snapshot_(std::make_shared<const AppConfig>()) {}

ConfigMonitor::~ConfigMonitor() { stop(); }

ConfigIoResult ConfigMonitor::start(const std::filesystem::path& path,
                                    const std::chrono::milliseconds interval) {
    std::unique_lock lock(state_mutex_);
    if (running_)
        return {false, false, false, false, generation_.load(),
                "configuration monitor is already running"};
    if (interval.count() < 10 || interval.count() > 5000)
        return {false, false, false, false, generation_.load(),
                "configuration monitor interval must be between 10 and 5000 ms"};
    path_ = path;
    interval_ = interval;
    auto loaded = store_.load(path_);
    snapshot_.store(std::make_shared<const AppConfig>(store_.snapshot()));
    generation_.store(store_.generation());
    diagnostic_ = loaded.diagnostic;
    running_ = true;
    ready_ = false;
    worker_ = std::jthread([this](const std::stop_token stop) { run(stop); });
    changed_.wait(lock, [&] { return ready_; });
    return loaded;
}

void ConfigMonitor::stop() {
    {
        std::lock_guard lock(state_mutex_);
        if (!running_) return;
        worker_.request_stop();
        changed_.notify_all();
    }
    if (worker_.joinable()) worker_.join();
    std::lock_guard lock(state_mutex_);
    running_ = false;
}

std::shared_ptr<const AppConfig> ConfigMonitor::snapshot() const noexcept {
    return snapshot_.load();
}

std::uint64_t ConfigMonitor::generation() const noexcept { return generation_.load(); }

std::string ConfigMonitor::last_diagnostic() const {
    std::lock_guard lock(state_mutex_);
    return diagnostic_;
}

bool ConfigMonitor::wait_for_generation(const std::uint64_t after,
                                        const std::chrono::milliseconds timeout,
                                        const std::stop_token stop) const {
    if (timeout.count() <= 0) return generation_.load() > after;
    std::unique_lock lock(state_mutex_);
    return changed_.wait_for(lock, stop, timeout,
                             [&] { return generation_.load() > after; });
}

void ConfigMonitor::run(const std::stop_token stop) {
    auto previous = observe(path_);
    {
        std::lock_guard lock(state_mutex_);
        ready_ = true;
    }
    changed_.notify_all();
    while (!stop.stop_requested()) {
        {
            std::unique_lock lock(state_mutex_);
            static_cast<void>(changed_.wait_for(lock, stop, interval_, [] { return false; }));
        }
        if (stop.stop_requested()) return;
        const auto current = observe(path_);
        if (current == previous) continue;
        previous = current;
        const auto result = store_.reload();
        {
            std::lock_guard lock(state_mutex_);
            diagnostic_ = result.diagnostic;
            if (result.success && result.changed) {
                snapshot_.store(std::make_shared<const AppConfig>(store_.snapshot()));
                generation_.store(result.generation);
            }
        }
        if (result.success && result.changed) changed_.notify_all();
    }
}

}  // namespace owo::config
