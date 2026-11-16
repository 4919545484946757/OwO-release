#include "owo/engine/lexicon.h"
#include "owo/ipc/named_pipe.h"
#include "owo/model/model_backend.h"
#include "owo/model/model_protocol.h"
#include "owo/protocol/messages.h"

#include <algorithm>
#include <chrono>
#include <iostream>
#include <thread>

namespace {
using namespace owo;
using namespace std::chrono_literals;

class RankingFixture final : public model::IModelBackend {
public:
    std::string_view id() const noexcept override { return "owo.test.zero"; }
    model::ModelResult rank(const model::ModelRequest& request, std::stop_token) override {
        if (request.context == "延时") return {request.request_id, model::ModelStatus::timeout, {}, {}};
        if (request.input == "xianzai")
            return {request.request_id, model::ModelStatus::success, {"在在"}, {}};
        auto rows = request.candidates;
        if (request.context == "去0号") {
            const auto target = std::find(rows.begin(), rows.end(), "西安");
            if (target != rows.end()) std::rotate(rows.begin(), target, target + 1);
        }
        return {request.request_id, model::ModelStatus::success, std::move(rows), {}};
    }
};
}

int main() {
    const engine::MemoryLexicon lexicon({
        {{"xian"}, "先", 1'000'000}, {{"xian"}, "现", 90'000},
        {{"xi", "an"}, "西安", 1000}, {{"xi"}, "西", 5'000}, {{"an"}, "安", 5'000},
        {{"xian", "zai"}, "现在", 500'000}, {{"zai"}, "在", 5'000},
        {{"xian", "zhai"}, "闲宅", 10'000'000}
    });
    const auto pipe = LR"(\\.\pipe\OwO.Test.Zero.)" +
        std::to_wstring(std::chrono::steady_clock::now().time_since_epoch().count());
    const auto model_pipe = pipe + L".Model";
    RankingFixture fixture;
    std::thread model_thread([&] { (void)ipc::run_model_server(model_pipe.c_str(), fixture); });
    std::thread core_thread([&] { (void)ipc::run_core_server(pipe.c_str(), lexicon, nullptr, model_pipe.c_str()); });
    bool ok = true;
    const auto require = [&ok](bool value, const char* issue) {
        if (!value) { std::cerr << issue << '\n'; ok = false; }
    };
    const auto send = [&](const protocol::Message& request) {
        const auto reply = ipc::exchange(pipe.c_str(), protocol::encode_message(request), 1s);
        const auto decoded = reply.status ? protocol::decode_message(reply.response) : protocol::DecodeResult{};
        require(reply.status && decoded.validation && decoded.message.request_id == request.request_id &&
            decoded.message.context_generation == request.context_generation, "invalid response or correlation id");
        return decoded.message;
    };
    const auto settle = [&](protocol::Message response) {
        protocol::Message poll;
        poll.type = protocol::MessageType::zero_candidate_update_request;
        poll.request_id = response.request_id;
        poll.context_generation = response.context_generation;
        for (int attempt = 0; response.model_pending && attempt < 60; ++attempt) {
            std::this_thread::sleep_for(20ms);
            response = send(poll);
        }
        require(!response.model_pending && response.type == protocol::MessageType::zero_candidate_response,
            "zero ranking did not settle");
        return response;
    };
    protocol::Message request;
    request.type = protocol::MessageType::zero_candidate_request;
    request.request_id = 1; request.context_generation = 5;
    request.input = "performance"; request.text = "xian"; request.context = "去３９号";
    const auto first = send(request);
    const auto has = [](const protocol::Message& response, std::string_view candidate) {
        return std::find(response.candidates.begin(), response.candidates.end(), candidate) != response.candidates.end();
    };
    require(first.model_pending && first.text == "先" && has(first, "西安") && !has(first, "西"),
        "lattice must preserve xian and xi'an, but not standalone prefix xi");
    protocol::Message ordinary;
    ordinary.type = protocol::MessageType::candidate_request;
    ordinary.request_id = 2; ordinary.context_generation = 5; ordinary.text = "xian";
    const auto visible = send(ordinary);
    require(visible.type == protocol::MessageType::candidate_response && !visible.candidates.empty(),
        "ordinary candidates must remain independent");
    protocol::Message mismatched;
    mismatched.type = protocol::MessageType::zero_candidate_update_request;
    mismatched.request_id = request.request_id;
    mismatched.context_generation = request.context_generation + 1;
    require(send(mismatched).text.empty(), "a stale context generation cannot read the ranking result");
    require(settle(first).text == "西安", "model should reorder an existing path using redacted context");

    request.request_id = 3; request.text = "xi'an"; request.context.clear();
    const auto manual = settle(send(request));
    require(manual.text == "西安", "must respect explicit apostrophe boundaries");

    request.request_id = 4; request.text = "xianzai";
    const auto exact = send(request);
    require(exact.text == "现在" && !has(exact, "现") && !has(exact, "在在") && !has(exact, "闲宅"),
        "only complete readings enter the shortlist and exact beats assisted");
    require(settle(exact).text == "现在", "model cannot inject an unlisted homophone");

    request.request_id = 5; request.text = "xian"; request.context = "延时";
    const auto deadline = send(request);
    require(settle(deadline).text == deadline.text && !deadline.text.empty(), "model timeout must keep local lattice top");

    request.request_id = 6; request.application_sensitive_input = true;
    require(send(request).text.empty(), "protected input must not request zero candidate");

    protocol::Message stop;
    stop.type = protocol::MessageType::shutdown_request;
    require(send(stop).type == protocol::MessageType::acknowledgement, "core shutdown failed");
    core_thread.join();
    model::ModelMessage stop_model;
    stop_model.type = model::ModelMessageType::shutdown_request;
    (void)ipc::exchange(model_pipe.c_str(), model::encode_model_message(stop_model), 1s);
    model_thread.join();
    return ok ? 0 : 1;
}
