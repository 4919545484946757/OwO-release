#pragma once

// Private, local-only diagnostic format. No text, paths, context or credentials.
// UI threads only enqueue bounded records; existing workers refresh/flush them.
// No new thread or DLL-unload lifetime is introduced into arbitrary TSF hosts.
#include <Windows.h>
#include <atomic>
#include <charconv>
#include <cstdint>
#include <filesystem>
#include <fstream>
#include <initializer_list>
#include <map>
#include <mutex>
#include <string>
#include <string_view>
#include <vector>

namespace owo::diagnostics {
struct TimingField { const char* name; std::int64_t value; };

inline std::uint64_t timing_now_us() noexcept {
    LARGE_INTEGER counter{}, frequency{};
    QueryPerformanceCounter(&counter);
    QueryPerformanceFrequency(&frequency);
    if (frequency.QuadPart <= 0) return 0;
    // Quotient/remainder avoids overflowing on machines with long uptime.
    return static_cast<std::uint64_t>(counter.QuadPart / frequency.QuadPart) * 1000000ULL +
        static_cast<std::uint64_t>((counter.QuadPart % frequency.QuadPart) * 1000000LL /
                                   frequency.QuadPart);
}

class CandidateTiming final {
public:
    static CandidateTiming& instance() noexcept { static CandidateTiming value; return value; }
    bool active() const noexcept { return enabled_.load(std::memory_order_relaxed); }

    // Worker-only. A missing/expired control file fails closed. Activation is
    // bounded to 30 minutes even if the controller crashes or exits abnormally.
    void refresh() noexcept {
        try {
            std::unique_lock maintenance(maintenance_mutex_, std::try_to_lock);
            if (!maintenance) return;
            const auto now = GetTickCount64();
            if (now < next_refresh_) return;
            next_refresh_ = now + 250;
            if (directory_.empty()) {
                wchar_t local[32768]{};
                const auto count = GetEnvironmentVariableW(L"LOCALAPPDATA", local, 32768);
                if (!count || count >= 32768) return;
                directory_ = std::filesystem::path(local) / L"OwO" / L"InputMethod" / L"diagnostics";
            }
            const auto control = directory_ / L"candidate-timing.control";
            std::error_code error;
            const auto bytes = std::filesystem::file_size(control, error);
            if (error || bytes > 256) { enabled_.store(false, std::memory_order_relaxed); return; }
            std::ifstream input(control, std::ios::binary);
            std::string session, expiry, owner, created;
            std::getline(input, session); std::getline(input, expiry);
            std::getline(input, owner); std::getline(input, created);
            std::uint64_t end{}, creation{};
            std::uint32_t pid{};
            const auto parsed = std::from_chars(expiry.data(), expiry.data() + expiry.size(), end);
            const auto owner_parsed = std::from_chars(owner.data(), owner.data() + owner.size(), pid);
            const auto creation_parsed = std::from_chars(created.data(), created.data() + created.size(), creation);
            bool valid = session.size() == 32 &&
                session.find_first_not_of("0123456789abcdef") == std::string::npos &&
                parsed.ec == std::errc{} && parsed.ptr == expiry.data() + expiry.size() &&
                end > now && end - now <= 30ULL * 60 * 1000 && pid != 0 &&
                owner_parsed.ec == std::errc{} && owner_parsed.ptr == owner.data() + owner.size() &&
                creation_parsed.ec == std::errc{} && creation_parsed.ptr == created.data() + created.size();
            // PID + process creation time prevents reuse after exit/reboot.
            if (valid) {
                const HANDLE controller = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, FALSE, pid);
                FILETIME born{}, exited{}, kernel{}, user{};
                valid = controller && GetProcessTimes(controller, &born, &exited, &kernel, &user) &&
                    ((static_cast<std::uint64_t>(born.dwHighDateTime) << 32) | born.dwLowDateTime) == creation &&
                    WaitForSingleObject(controller, 0) == WAIT_TIMEOUT;
                if (controller) CloseHandle(controller);
            }
            std::lock_guard lock(mutex_);
            session_ = valid ? session : std::string{};
            enabled_.store(valid, std::memory_order_relaxed);
        } catch (...) { enabled_.store(false, std::memory_order_relaxed); }
    }

    void emit(std::uint64_t request, const char* component, const char* event,
              std::initializer_list<TimingField> fields = {}) noexcept {
        if (!active()) return;
        try {
            const auto timestamp = timing_now_us();
            std::string session;
            { std::lock_guard lock(mutex_); session = session_; }
            if (session.empty()) return;
            // Format outside the queue lock. The only UI-thread contention is
            // a short session copy/vector push, never filesystem operations.
            std::string row = "{\"schema\":1,\"session\":\"" + session +
                "\",\"pid\":" + std::to_string(GetCurrentProcessId()) +
                ",\"request_id\":" + std::to_string(request) +
                ",\"timestamp_us\":" + std::to_string(timestamp) +
                ",\"component\":\"" + component + "\",\"event\":\"" + event + "\"";
            for (const auto& field : fields)
                row += ",\"" + std::string(field.name) + "\":" + std::to_string(field.value);
            row += ",\"dropped_total\":" + std::to_string(dropped_.load()) + "}\n";
            std::lock_guard lock(mutex_);
            if (pending_.size() >= 1024) { ++dropped_; return; }
            pending_.push_back({std::move(session), std::move(row)});
        } catch (...) { ++dropped_; }
    }

    // Worker-only, never called from a TSF UI event. No FlushFileBuffers: disk
    // durability must not be inserted into the measured input/render path.
    void flush() noexcept {
        try {
            std::lock_guard maintenance(maintenance_mutex_);
            std::vector<Record> records;
            { std::lock_guard lock(mutex_);
              if (pending_.empty()) return;
              records.swap(pending_); }
            std::map<std::string, std::string> batches;
            for (auto& record : records) batches[record.session] += record.row;
            for (const auto& [session, bytes] : batches) {
                const auto file = directory_ / (session + "-" + std::to_string(GetCurrentProcessId()) + ".jsonl");
                const HANDLE handle = CreateFileW(file.c_str(), FILE_APPEND_DATA | FILE_READ_ATTRIBUTES,
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, nullptr, OPEN_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL, nullptr);
                if (handle == INVALID_HANDLE_VALUE) { dropped_ += records.size(); continue; }
                LARGE_INTEGER size{};
                DWORD written{};
                const bool ok = GetFileSizeEx(handle, &size) && size.QuadPart >= 0 &&
                    static_cast<std::uint64_t>(size.QuadPart) + bytes.size() <= 8ULL * 1024 * 1024 &&
                    WriteFile(handle, bytes.data(), static_cast<DWORD>(bytes.size()), &written, nullptr) &&
                    written == bytes.size();
                if (!ok) dropped_ += records.size();
                CloseHandle(handle);
            }
        } catch (...) { ++dropped_; }
    }
private:
    struct Record { std::string session, row; };
    std::atomic<bool> enabled_{};
    std::atomic<std::uint64_t> dropped_{};
    std::mutex mutex_;
    std::mutex maintenance_mutex_;
    std::vector<Record> pending_;
    std::filesystem::path directory_;
    std::string session_;
    std::uint64_t next_refresh_{};
};

inline void candidate_timing(std::uint64_t request, const char* component, const char* event,
                             std::initializer_list<TimingField> fields = {}) noexcept {
    CandidateTiming::instance().emit(request, component, event, fields);
}
} // namespace owo::diagnostics
