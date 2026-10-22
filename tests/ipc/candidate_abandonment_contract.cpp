#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <Windows.h>
#include <atomic>
#include <condition_variable>
#include <iostream>
#include <mutex>
#include <thread>

namespace {
// Simulate a descheduled/disk-faulted lookup, not a CPU-speed-dependent sleep.
class PausedLexicon final : public owo::engine::Lexicon {
public:
    bool wait_until_paused() const {
        std::unique_lock lock(mutex_);
        return ready_.wait_for(lock, std::chrono::seconds(2), [&] { return paused_; });
    }
    void release() const {
        std::lock_guard lock(mutex_);
        released_ = true;
        ready_.notify_all();
    }
    std::vector<owo::engine::LexiconEntry> lookup(std::span<const std::string_view> s) const override { return base_.lookup(s); }
    std::vector<owo::engine::LexiconTextEntry> lookup_texts(std::span<const std::string_view> s) const override { return base_.lookup_texts(s); }
    std::uint32_t lookup_max_frequency(std::span<const std::string_view> s) const override {
        if (++lookups == 1) {
            std::unique_lock lock(mutex_);
            paused_ = true;
            ready_.notify_all();
            ready_.wait_for(lock, std::chrono::seconds(2), [&] { return released_; });
        }
        return base_.lookup_max_frequency(s);
    }
    std::vector<owo::engine::LexiconEntry> lookup_initial(char c, std::size_t n) const override { return base_.lookup_initial(c,n); }
    std::vector<owo::engine::InitialLexiconTextEntry> lookup_initial_texts(char c, std::size_t n) const override { return base_.lookup_initial_texts(c,n); }
    std::vector<owo::engine::AbbreviatedLexiconMatch> lookup_mixed_abbreviation(std::string_view s, std::size_t n) const override { return base_.lookup_mixed_abbreviation(s,n); }
    std::vector<owo::engine::AbbreviatedLexiconTextMatch> lookup_mixed_abbreviation_texts(std::string_view s, std::size_t n) const override { return base_.lookup_mixed_abbreviation_texts(s,n); }
    std::size_t maximum_reading_length() const noexcept override { return base_.maximum_reading_length(); }
    mutable std::atomic<unsigned> lookups{};
private:
    owo::engine::MemoryLexicon base_{{{{"ni", "hao"}, "你好", 1000},
        {{"shi", "jie"}, "世界", 900}, {{"ni"}, "你", 800}, {{"hao"}, "好", 800},
        {{"shi"}, "是", 800}, {{"jie"}, "界", 800}}};
    mutable std::mutex mutex_;
    mutable std::condition_variable ready_;
    mutable bool paused_{}, released_{};
};
}

int main() {
    for (const bool event_cancel : {false, true}) {
        PausedLexicon lexicon;
        const auto pipe = L"\\\\.\\pipe\\OwO.CandidateAbandon." +
            std::to_wstring(GetCurrentProcessId()) + (event_cancel ? L".event" : L".timeout");
        int server_exit = -1;
        std::thread server([&] { server_exit = owo::ipc::run_core_server(pipe.c_str(), lexicon); });
        owo::protocol::Message request;
        request.type = owo::protocol::MessageType::candidate_request;
        request.request_id = 911;
        request.context_generation = GetCurrentProcessId();
        request.text = "nihaoshijienihaoshijie";
        const auto name = owo::ipc::candidate_cancellation_event_name(
            request.request_id, request.context_generation);
        const HANDLE event = event_cancel ? CreateEventW(nullptr, TRUE, FALSE, name.c_str()) : nullptr;
        owo::ipc::ExchangeResult result;
        std::thread client([&] { result = owo::ipc::exchange(pipe.c_str(),
            owo::protocol::encode_message(request), std::chrono::milliseconds(event_cancel ? 1500 : 300)); });
        bool ok = lexicon.wait_until_paused();
        if (event_cancel) {
            SetEvent(event);
            lexicon.release();
            client.join();
        } else {
            client.join(); // Timeout closes the pipe, with no cancellation event.
            ok = ok && !result.status;
            lexicon.release();
        }
        request.request_id++;
        request.type = owo::protocol::MessageType::candidate_update_request;
        const auto ping = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(request),
            std::chrono::milliseconds(1500));
        ok = ok && ping.status && lexicon.lookups.load() == 1;
        request.type = owo::protocol::MessageType::candidate_request;
        request.text = "nihao";
        const auto next = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(request),
            std::chrono::milliseconds(1500));
        const auto decoded = owo::protocol::decode_message(next.response);
        ok = ok && next.status && decoded.validation && !decoded.message.candidates.empty() &&
            decoded.message.candidates.front() == "你好";
        request.type = owo::protocol::MessageType::shutdown_request;
        const auto stopped = owo::ipc::exchange(pipe.c_str(), owo::protocol::encode_message(request),
            std::chrono::milliseconds(1500));
        server.join();
        if (event) CloseHandle(event);
        if (!ok || !stopped.status || server_exit != 0) {
            std::cerr << "Abandoned candidate blocked next request: event=" << event_cancel << '\n';
            return 1;
        }
    }
    std::cout << "Timed-out/disconnected and superseded requests stop before further lookups\n";
}
