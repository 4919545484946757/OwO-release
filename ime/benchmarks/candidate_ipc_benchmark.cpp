#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/protocol/messages.h"

#include <Windows.h>
#include <algorithm>
#include <chrono>
#include <iostream>
#include <thread>
#include <vector>

namespace {
using Clock = std::chrono::steady_clock;
struct Samples {
    std::vector<double> values;
    void add(Clock::time_point start) { values.push_back(std::chrono::duration<double,std::milli>(Clock::now()-start).count()); }
    void print() {
        std::sort(values.begin(),values.end());
        if (values.empty()) { std::cout << "null"; return; }
        std::cout << "{\"p50\":" << values[(values.size()-1)/2]
            << ",\"p95\":" << values[(values.size()*95+99)/100-1]
            << ",\"max\":" << values.back() << '}';
    }
};
bool exchange(owo::ipc::PersistentPipeClient& client, const owo::protocol::Message& request,
              owo::protocol::Message& response, owo::protocol::MessageType expected) {
    const auto result = client.exchange(owo::protocol::encode_message(request),std::chrono::milliseconds(2500));
    if (!result.status) { std::cerr << "IPC probe: " << result.status.message << '\n'; return false; }
    auto decoded = owo::protocol::decode_message(result.response);
    if (!decoded.validation || decoded.message.type != expected ||
        decoded.message.request_id != request.request_id ||
        decoded.message.context_generation != request.context_generation) {
        std::cerr << "IPC probe: invalid/stale response: " << decoded.validation.message << '\n';
        return false;
    }
    response = std::move(decoded.message);
    return true;
}
owo::protocol::Message ranking_request(const owo::protocol::Message& candidate) {
    // Model polls have a smaller protocol contract; do not copy first-stage
    // defer/refinement flags or input fields into them.
    return {owo::protocol::MessageType::candidate_update_request,
        candidate.request_id, candidate.context_generation, {}};
}
int measure(const std::wstring& pipe, unsigned count, bool wait_model) {
    using namespace owo::protocol;
    unsigned failures = 0;
    std::uint64_t id = GetTickCount64();
    for (const auto query : {"nihao","nihaoshijie","rwkyy","gongnnegkaiguan",
                             "wangwangshituobukuaigauanxi"}) {
        Samples initial, refinement, cached, ranking;
        unsigned empty = 0, empty_final = 0, ranked = 0;
        for (unsigned sample = 0; sample < count; ++sample) {
            owo::ipc::PersistentPipeClient client(pipe);
            Message request, response;
            request.type = MessageType::candidate_request;
            request.request_id = ++id;
            request.context_generation = (static_cast<std::uint64_t>(GetCurrentProcessId()) << 32) | 0x50455246U;
            request.text = query;
            request.defer_expensive_candidates = true;
            auto start = Clock::now();
            if (!exchange(client,request,response,MessageType::candidate_response)) { ++failures; continue; }
            initial.add(start);
            if (response.candidates.empty()) ++empty;
            if (response.candidate_refinement_pending) {
                request.defer_expensive_candidates = false;
                start = Clock::now();
                if (!exchange(client,request,response,MessageType::candidate_response)) { ++failures; continue; }
                refinement.add(start);
            }
            if (wait_model && response.model_pending) {
                const auto update = ranking_request(request);
                start = Clock::now();
                const auto deadline = start + std::chrono::milliseconds(500);
                while (Clock::now() < deadline) {
                    if (!exchange(client,update,response,MessageType::candidate_update_response)) { ++failures; break; }
                    if (!response.model_pending) {
                        if (!response.candidates.empty()) { ++ranked; ranking.add(start); }
                        break;
                    }
                    std::this_thread::sleep_for(std::chrono::milliseconds(2));
                }
            }
            request.request_id = ++id;
            request.defer_expensive_candidates = false;
            start = Clock::now();
            if (!exchange(client,request,response,MessageType::candidate_response)) { ++failures; continue; }
            cached.add(start);
            if (response.candidates.empty()) ++empty_final;
        }
        std::cout << "{\"benchmark\":\"owo.ipc.candidate_latency\",\"query\":\"" << query
            << "\",\"requested_samples\":" << count << ",\"completed_samples\":" << cached.values.size()
            << ",\"empty_first_responses\":" << empty << ",\"initial_roundtrip_ms\":";
        initial.print(); std::cout << ",\"refinement_roundtrip_ms\":";
        refinement.print(); std::cout << ",\"cached_roundtrip_ms\":";
        cached.print(); std::cout << ",\"empty_final_responses\":" << empty_final
            << ",\"ranked_responses\":" << ranked << ",\"ranking_wait_ms\":";
        ranking.print(); std::cout << ",\"failures_total\":" << failures << "}\n";
    }
    return failures ? 1 : 0;
}
}

int wmain(int argc, wchar_t** argv) {
    bool run = false, self_test = false, wait_model = false;
    unsigned samples = 7;
    std::wstring pipe = owo::ipc::kCorePipeName;
    for (int i=1; i<argc; ++i) {
        const std::wstring_view arg(argv[i]);
        if (arg == L"--run") run = true;
        else if (arg == L"--self-test") self_test = true;
        else if (arg == L"--wait-model") wait_model = true;
        else if (arg == L"--samples" && i+1<argc) {
            try { samples = std::stoul(argv[++i]); } catch (...) { return 2; }
            if (samples == 0 || samples > 100) return 2;
        } else if (arg == L"--pipe" && i+1<argc) pipe = argv[++i];
        else if (arg != L"--help") return 2;
    }
    if (!run && !self_test) {
        std::cout << "Usage: owo_candidate_ipc_benchmark --run [--samples 7] [--wait-model]\n"
            "Sends fixed synthetic queries only. No learning feedback, context collection or file writes.\n"
            "Run while not typing. First roundtrip includes connect, not guaranteed cold storage.\n";
        return 0;
    }
    if (!self_test) return measure(pipe,samples,wait_model);
    // A private in-process Core makes the benchmark's contract test independent
    // of the user's installed Core/configuration/learning store.
    pipe = L"\\\\.\\pipe\\OwO.CandidatePerf.Contract." + std::to_wstring(GetCurrentProcessId());
    owo::engine::MemoryLexicon fixture({{{"ni","hao"},"你好",1000},
        {{"shi","jie"},"世界",900},{{"ni"},"你",800},{{"hao"},"好",800}});
    int server_exit = -1;
    std::thread server([&] { server_exit = owo::ipc::run_core_server(pipe.c_str(),fixture); });
    const int result = measure(pipe,samples,false);
    owo::protocol::Message shutdown, response;
    owo::protocol::Message unresolved;
    unresolved.type = owo::protocol::MessageType::candidate_request;
    unresolved.request_id = GetTickCount64();
    unresolved.context_generation = GetCurrentProcessId();
    unresolved.text = "wangwangshituobukuaigauanxi";
    unresolved.defer_expensive_candidates = true;
    bool unresolved_ok = true;
    for (const bool correction : {false,true}) {
        owo::ipc::PersistentPipeClient preview_client(pipe);
        unresolved.correction_enabled = correction;
        unresolved_ok = exchange(preview_client,unresolved,response,
            owo::protocol::MessageType::candidate_response) && response.syllables.empty() && unresolved_ok;
    }
    {
        owo::ipc::PersistentPipeClient poll_client(pipe);
        unresolved_ok = exchange(poll_client,ranking_request(unresolved),response,
            owo::protocol::MessageType::candidate_update_response) && unresolved_ok;
    }
    shutdown.type = owo::protocol::MessageType::shutdown_request;
    owo::ipc::PersistentPipeClient client(pipe);
    const bool stopped = exchange(client,shutdown,response,owo::protocol::MessageType::acknowledgement);
    client.reset(); server.join();
    return stopped && unresolved_ok && server_exit == 0 ? result : 1;
}
