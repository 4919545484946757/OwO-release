#include "owo/ipc/input_method_client.h"
#include "owo/ipc/input_method_control.h"
#include "owo/ipc/named_pipe.h"
#include "owo/plugin/input_method_protocol.h"
#include <algorithm>
#include <condition_variable>
#include <mutex>
#include <thread>

namespace owo::ipc {
namespace {
using namespace std::chrono_literals;
using Clock = std::chrono::steady_clock;
using Error = protocol::ErrorCode;
InputMethodClientResult failure(std::uint64_t generation, Error code) {
    const char* message = code == Error::cancelled ? "Input scheme request cancelled" :
        code == Error::timeout ? "Input scheme request timed out" :
        code == Error::invalid_payload ? "Invalid input scheme request or response" :
        "Input scheme connection unavailable";
    return {generation, {code, message}, {}, {}};
}
std::future<InputMethodClientResult> immediate(InputMethodClientResult result) {
    std::promise<InputMethodClientResult> p; auto future = p.get_future();
    p.set_value(std::move(result)); return future;
}
struct Failed { Error code; };
}
struct InputMethodAsyncClient::Impl {
    enum class Action { discover, open, submit, close };
    struct Job {
        Action action{};
        std::uint64_t generation{}, epoch{};
        std::string plugin, scheme, session;
        engine::InputMethodRequest input;
        std::stop_source cancellation;
        std::promise<InputMethodClientResult> promise;
    };
    PersistentPipeClient pipe;
    std::mutex mutex;
    std::condition_variable cv;
    bool stopping{};
    std::optional<Job> pending;
    std::stop_source active;
    std::uint64_t epoch{}, generation{}, last_local_id{};
    std::optional<engine::InputMethodDescriptor> advertised;
    std::string session;
    // Worker-owned transport sequence and binding. Poll IDs must not consume UI request IDs.
    std::uint64_t wire_id{}, bound_generation{};
    std::optional<engine::InputMethodDescriptor> bound;
    std::thread worker;
    explicit Impl(std::wstring name) : pipe(std::move(name)), worker([this] { loop(); }) {}
    ~Impl() {
        {
            std::lock_guard lock(mutex); stopping = true; ++epoch; active.request_stop();
            if (pending) { pending->promise.set_value(failure(pending->generation, Error::cancelled)); pending.reset(); }
        }
        cv.notify_all(); worker.join();
    }
    std::future<InputMethodClientResult> enqueue(Job next) {
        auto result = next.promise.get_future();
        active.request_stop();
        if (pending) pending->promise.set_value(failure(pending->generation, Error::cancelled));
        next.epoch = ++epoch; pending = std::move(next); cv.notify_all(); return result;
    }
    InputMethodControlResponse rpc(InputMethodControlRequest request, std::stop_token stop, Clock::time_point deadline) {
        if (stop.stop_requested()) throw Failed{Error::cancelled};
        const auto remaining = std::chrono::duration_cast<std::chrono::milliseconds>(deadline - Clock::now());
        if (remaining <= 0ms) throw Failed{Error::timeout};
        const auto wire = encode_input_method_control_request(request);
        if (wire.empty()) throw Failed{Error::invalid_payload};
        auto exchanged = pipe.exchange(wire, (std::min)(remaining, 200ms));
        if (stop.stop_requested()) throw Failed{Error::cancelled};
        if (!exchanged.status) throw Failed{exchanged.status.error};
        InputMethodControlResponse response;
        if (!decode_input_method_control_response(exchanged.response, response) || response.request_id != request.request_id)
            throw Failed{Error::invalid_payload};
        return response;
    }
    InputMethodControlResponse complete(InputMethodControlRequest request, std::stop_token stop) {
        const auto deadline = Clock::now() + 2s;
        auto response = rpc(request, stop, deadline);
        const auto ticket = request.request_id;
        while (response.status == InputMethodControlStatus::pending) {
            if (response.ticket != ticket) throw Failed{Error::invalid_payload};
            {
                std::unique_lock lock(mutex);
                cv.wait_for(lock, 10ms, [&] { return stopping || stop.stop_requested(); });
            }
            InputMethodControlRequest poll;
            poll.request_id = ++wire_id; poll.action = InputMethodControlAction::poll; poll.ticket = ticket;
            response = rpc(poll, stop, deadline);
        }
        if (response.status != InputMethodControlStatus::ready) throw Failed{Error::transport_unavailable};
        if (response.ticket != ticket) throw Failed{Error::invalid_payload};
        return response;
    }
    InputMethodClientResult run(const Job& j) {
        const auto stop = j.cancellation.get_token();
        InputMethodClientResult result{j.generation, {}, {}, {}};
        if (j.action == Action::close) { pipe.reset(); bound.reset(); bound_generation = 0; return result; }
        InputMethodControlRequest request; request.request_id = ++wire_id;
        if (j.action == Action::submit) {
            if (!bound || bound_generation != j.generation || j.input.scheme_id != bound->id)
                throw Failed{Error::transport_unavailable};
            auto input = j.input; input.request_id = request.request_id;
            request.action = InputMethodControlAction::submit;
            request.payload = plugin::encode_input_method_request(*bound, input);
            const auto response = complete(request, stop);
            engine::InputMethodResponse value;
            if (!plugin::decode_input_method_response(*bound, input, response.payload, value).ok)
                throw Failed{Error::invalid_payload};
            // Restore the local request ID only after validating the complete wire identity.
            value.request_id = j.input.request_id; result.response = std::move(value);
        } else {
            pipe.reset(); bound.reset(); bound_generation = 0;
            request.plugin_id = j.plugin;
            request.action = j.action == Action::open ? InputMethodControlAction::open : InputMethodControlAction::discover;
            if (j.action == Action::open) { request.scheme_id = j.scheme; request.session_id = j.session; }
            const auto response = complete(request, stop);
            if (!plugin::decode_input_method_descriptors(j.plugin, response.payload, result.descriptors).ok)
                throw Failed{Error::invalid_payload};
            if (j.action == Action::open) {
                if (result.descriptors.size() != 1 || result.descriptors.front().id != j.scheme)
                    throw Failed{Error::invalid_payload};
                bound = result.descriptors.front(); bound_generation = j.generation;
            }
        }
        return result;
    }
    void loop() {
        for (;;) {
            std::optional<Job> current;
            {
                std::unique_lock lock(mutex); cv.wait(lock, [&] { return stopping || pending.has_value(); });
                if (stopping) break;
                current = std::move(pending); pending.reset(); active = current->cancellation;
            }
            auto& j = *current;
            InputMethodClientResult result;
            try { result = run(j); }
            catch (const Failed& e) { result = failure(j.generation, e.code); }
            catch (...) { result = failure(j.generation, Error::transport_unavailable); }
            if (!result.status && result.status.error != Error::cancelled) {
                pipe.reset(); bound.reset(); bound_generation = 0;
            }
            {
                std::lock_guard lock(mutex);
                if (stopping || j.epoch != epoch || j.cancellation.stop_requested())
                    result = failure(j.generation, Error::cancelled);
                else if (j.action == Action::open && result.status) advertised = result.descriptors.front();
                else if (!result.status) advertised.reset();
                result.operation = j.action == Action::open ? InputMethodClientOperation::open :
                    j.action == Action::discover ? InputMethodClientOperation::discover :
                    j.action == Action::submit ? InputMethodClientOperation::submit : InputMethodClientOperation::close;
                j.promise.set_value(std::move(result));
            }
        }
        pipe.reset();
    }
};
InputMethodAsyncClient::InputMethodAsyncClient(std::wstring name) : impl_(std::make_unique<Impl>(std::move(name))) {}
InputMethodAsyncClient::~InputMethodAsyncClient() = default;
std::future<InputMethodClientResult> InputMethodAsyncClient::discover(std::uint64_t gen, std::string plugin) {
    InputMethodControlRequest probe; probe.request_id = 1; probe.plugin_id = plugin;
    if (!gen || encode_input_method_control_request(probe).empty()) return immediate(failure(gen, Error::invalid_payload));
    std::lock_guard lock(impl_->mutex);
    impl_->advertised.reset(); impl_->generation = gen; impl_->session.clear(); impl_->last_local_id = 0;
    Impl::Job job; job.action = Impl::Action::discover; job.generation = gen; job.plugin = std::move(plugin);
    return impl_->enqueue(std::move(job));
}
std::future<InputMethodClientResult> InputMethodAsyncClient::open(std::uint64_t gen, std::string plugin,
    std::string scheme, std::string session, bool sensitive) {
    if (sensitive) { (void)close(); return immediate(failure(gen, Error::invalid_payload)); }
    InputMethodControlRequest probe; probe.request_id = 1; probe.action = InputMethodControlAction::open;
    probe.plugin_id = plugin; probe.scheme_id = scheme; probe.session_id = session;
    if (!gen || encode_input_method_control_request(probe).empty()) return immediate(failure(gen, Error::invalid_payload));
    std::lock_guard lock(impl_->mutex);
    impl_->advertised.reset(); impl_->generation = gen; impl_->session = session; impl_->last_local_id = 0;
    Impl::Job job; job.action = Impl::Action::open; job.generation = gen;
    job.plugin = std::move(plugin); job.scheme = std::move(scheme); job.session = std::move(session);
    return impl_->enqueue(std::move(job));
}
std::future<InputMethodClientResult> InputMethodAsyncClient::submit(std::uint64_t gen, engine::InputMethodRequest input, bool sensitive) {
    if (sensitive) { (void)close(); return immediate(failure(gen, Error::invalid_payload)); }
    std::lock_guard lock(impl_->mutex);
    if (!gen || gen != impl_->generation || !impl_->advertised || input.session_id != impl_->session ||
        input.request_id <= impl_->last_local_id || !engine::validate_input_method_request(*impl_->advertised, input).ok)
        return immediate(failure(gen, Error::invalid_payload));
    impl_->last_local_id = input.request_id;
    Impl::Job job; job.action = Impl::Action::submit; job.generation = gen; job.input = std::move(input);
    return impl_->enqueue(std::move(job));
}
std::future<InputMethodClientResult> InputMethodAsyncClient::close() {
    std::lock_guard lock(impl_->mutex);
    impl_->advertised.reset(); impl_->generation = 0; impl_->session.clear(); impl_->last_local_id = 0;
    Impl::Job job; job.action = Impl::Action::close;
    return impl_->enqueue(std::move(job));
}
} // namespace owo::ipc
