#pragma once

#include "owo/config/config_store.h"

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <filesystem>
#include <memory>
#include <mutex>
#include <stop_token>
#include <string>
#include <thread>

namespace owo::config {

class ConfigMonitor final {
public:
    ConfigMonitor();
    ConfigMonitor(const ConfigMonitor&) = delete;
    ConfigMonitor& operator=(const ConfigMonitor&) = delete;
    ~ConfigMonitor();

    /// 同步完成首次加载后启动后台监控；轮询间隔只用于非按键后台线程。
    [[nodiscard]] ConfigIoResult start(const std::filesystem::path& path,
                                       std::chrono::milliseconds interval =
                                           std::chrono::milliseconds(250));
    void stop();
    [[nodiscard]] std::shared_ptr<const AppConfig> snapshot() const noexcept;
    [[nodiscard]] std::uint64_t generation() const noexcept;
    [[nodiscard]] std::string last_diagnostic() const;
    [[nodiscard]] bool wait_for_generation(std::uint64_t after,
                                           std::chrono::milliseconds timeout,
                                           std::stop_token stop = {}) const;

private:
    void run(std::stop_token stop);

    ConfigStore store_;
    std::filesystem::path path_;
    std::chrono::milliseconds interval_{250};
    std::atomic<std::shared_ptr<const AppConfig>> snapshot_;
    std::atomic<std::uint64_t> generation_{};
    mutable std::mutex state_mutex_;
    mutable std::condition_variable_any changed_;
    std::string diagnostic_;
    std::jthread worker_;
    bool running_{};
    bool ready_{};
};

}  // namespace owo::config
