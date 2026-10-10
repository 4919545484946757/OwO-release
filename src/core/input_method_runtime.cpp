#include "owo/core/input_method_runtime.h"

#include "owo/plugin/input_method_protocol.h"
#include "owo/plugin/plugin_authorization_store.h"
#include "owo/plugin/plugin_host.h"
#include "owo/plugin/plugin_store.h"

#include <algorithm>
#include <condition_variable>
#include <mutex>
#include <optional>
#include <thread>

namespace owo::core {
namespace {
using namespace engine;
InputMethodResponse failure(const InputMethodRequest& r, InputMethodStatus status, std::string message = {}) {
    InputMethodResponse v;
    v.request_id = r.request_id; v.session_id = r.session_id; v.scheme_id = r.scheme_id;
    v.page = r.page; v.status = status; v.diagnostic = std::move(message);
    return v;
}
class ProcessProvider final : public InputMethodProvider {
public:
    ProcessProvider(std::filesystem::path root, InputMethodDescriptor d,
                    plugin::InstalledPluginVersionResult binding, std::chrono::milliseconds timeout)
        : root_(std::move(root)), descriptor_(std::move(d)), binding_(std::move(binding)), timeout_(timeout) {}

    InputMethodResponse handle(const InputMethodRequest& r, std::stop_token stop) override {
        // Registry may be dispatched by multiple callers, but a process accepts one invocation.
        std::lock_guard lock(mutex_);
        const auto reject = [&](InputMethodStatus status, std::string diagnostic = {}) {
            issued_ids_.clear();
            return failure(r, status, std::move(diagnostic));
        };
        if (stop.stop_requested()) return reject(InputMethodStatus::cancelled);
        if (!authorized()) {
            session_.terminate();
            return reject(InputMethodStatus::unavailable, "Input scheme authorization unavailable");
        }
        if (r.action == InputMethodAction::select && (r.session_id != issued_session_ ||
            r.input != issued_input_ || r.page != issued_page_ ||
            std::find(issued_ids_.begin(), issued_ids_.end(), r.selected_candidate_id) == issued_ids_.end()))
            return reject(InputMethodStatus::error, "Candidate selection is stale or unknown");
        issued_ids_.clear();
        const auto payload = plugin::encode_input_method_request(descriptor_, r);
        if (payload.empty()) return failure(r, InputMethodStatus::error, "Invalid scheme request");
        const auto deadline = std::chrono::steady_clock::now() + timeout_;
        if (!session_.valid()) {
            auto launched = plugin::launch_active_plugin(root_, descriptor_.plugin_id, timeout_);
            if (!launched) return failure(r, InputMethodStatus::unavailable, "Input scheme process unavailable");
            session_ = std::move(launched.session);
        }
        if (stop.stop_requested()) return failure(r, InputMethodStatus::cancelled);
        const auto remaining = std::chrono::duration_cast<std::chrono::milliseconds>(
            deadline - std::chrono::steady_clock::now());
        if (remaining.count() <= 0) return failure(r, InputMethodStatus::error, "Input scheme timed out");
        plugin::PluginInvokeRequest call;
        call.service = plugin::kInputMethodService;
        call.payload = payload;
        call.required_permissions = {"input.context"};
        // Attributed by the host-selected scheme, never decoded from plugin or Core IPC data.
        call.user_initiated = true;
        call.timeout = remaining;
        call.cancellation = stop;
        const auto invoked = session_.invoke(std::move(call));
        if (stop.stop_requested() || invoked.status == plugin::PluginStatus::cancelled)
            return failure(r, InputMethodStatus::cancelled);
        if (!invoked.ok) return failure(r, InputMethodStatus::error,
            invoked.status == plugin::PluginStatus::timeout ? "Input scheme timed out" : "Input scheme invocation failed");
        // Disable, update or revoke during computation invalidates the result as well.
        if (!authorized()) {
            session_.terminate();
            return failure(r, InputMethodStatus::unavailable, "Input scheme authorization changed");
        }
        InputMethodResponse response;
        if (!plugin::decode_input_method_response(descriptor_, r, invoked.payload, response).ok) {
            session_.terminate();
            return failure(r, InputMethodStatus::error, "Invalid input scheme response");
        }
        if (!stop.stop_requested() && response.status == InputMethodStatus::success) {
            issued_session_ = r.session_id; issued_input_ = r.input; issued_page_ = r.page;
            for (const auto& candidate : response.candidates) issued_ids_.push_back(candidate.id);
        }
        return response;
    }
private:
    bool authorized() const {
        const auto active = plugin::query_active_plugin_version(root_, descriptor_.plugin_id);
        if (!active.ok || active.installed_path != binding_.installed_path ||
            active.manifest.version != binding_.manifest.version ||
            active.inventory_sha256 != binding_.inventory_sha256 ||
            active.publisher_certificate_sha256 != binding_.publisher_certificate_sha256) return false;
        const auto grant = plugin::load_plugin_authorization(root_, descriptor_.plugin_id, active.manifest.version);
        return grant.ok && plugin::is_plugin_permission_granted(grant.value, active.manifest,
            active.inventory_sha256, active.publisher_certificate_sha256, "input.context");
    }
    std::filesystem::path root_;
    InputMethodDescriptor descriptor_;
    plugin::InstalledPluginVersionResult binding_;
    std::chrono::milliseconds timeout_;
    std::mutex mutex_;
    plugin::PluginHostSession session_;
    std::string issued_session_;
    std::string issued_input_;
    std::size_t issued_page_{};
    std::vector<std::string> issued_ids_;
};
} // namespace

engine::InputMethodValidation register_plugin_input_method(engine::InputMethodRegistry& registry,
    const std::filesystem::path& root, engine::InputMethodDescriptor descriptor, std::chrono::milliseconds timeout) {
    const auto validated = engine::validate_input_method_descriptor(descriptor);
    if (!validated.ok) return validated;
    if (timeout.count() <= 0 || timeout > std::chrono::seconds(5)) return {false, "Invalid scheme timeout"};
    const auto active = plugin::query_active_plugin_version(root, descriptor.plugin_id);
    if (!active.ok || active.manifest.runtime != "process") return {false, "Active process plugin required"};
    const auto grant = plugin::load_plugin_authorization(root, descriptor.plugin_id, active.manifest.version);
    if (!grant.ok || !plugin::is_plugin_permission_granted(grant.value, active.manifest,
        active.inventory_sha256, active.publisher_certificate_sha256, "input.context"))
        return {false, "Explicit input.context authorization required"};
    auto provider = std::make_shared<ProcessProvider>(root, descriptor, active, timeout);
    return registry.add(std::move(descriptor), std::move(provider));
}

struct InputMethodWorker::Impl {
    struct Job {
        InputMethodRequest request;
        std::promise<InputMethodResponse> promise;
        std::uint64_t generation{};
    };
    explicit Impl(InputMethodRegistry& value) : registry(value),
        worker([this](std::stop_token stop) { run(stop); }) {}
    ~Impl() {
        worker.request_stop();
        std::stop_source cancellation;
        {
            std::lock_guard lock(mutex);
            ++generation;
            cancellation = active;
            if (pending) {
                pending->promise.set_value(failure(pending->request, InputMethodStatus::cancelled));
                pending.reset();
            }
        }
        cancellation.request_stop();
        condition.notify_all();
        if (worker.joinable()) worker.join();
    }
    void run(std::stop_token stop) {
        for (;;) {
            Job job;
            std::stop_token token;
            {
                std::unique_lock lock(mutex);
                condition.wait(lock, stop, [&] { return pending.has_value(); });
                if (stop.stop_requested()) return;
                job = std::move(*pending);
                pending.reset();
                active = std::stop_source{};
                token = active.get_token();
            }
            auto response = registry.dispatch(job.request, token);
            {
                std::lock_guard lock(mutex);
                if (generation != job.generation || stop.stop_requested())
                    response = failure(job.request, InputMethodStatus::cancelled);
                job.promise.set_value(std::move(response));
            }
        }
    }
    InputMethodRegistry& registry;
    std::mutex mutex;
    std::condition_variable_any condition;
    std::optional<Job> pending;
    std::stop_source active;
    std::uint64_t generation{};
    std::jthread worker;
};

InputMethodWorker::InputMethodWorker(engine::InputMethodRegistry& registry)
    : implementation_(std::make_unique<Impl>(registry)) {}
InputMethodWorker::~InputMethodWorker() = default;

std::future<engine::InputMethodResponse> InputMethodWorker::submit(engine::InputMethodRequest request) {
    Impl::Job job;
    job.request = std::move(request);
    auto future = job.promise.get_future();
    std::stop_source cancellation;
    {
        std::lock_guard lock(implementation_->mutex);
        job.generation = ++implementation_->generation;
        cancellation = implementation_->active;
        if (implementation_->pending)
            implementation_->pending->promise.set_value(failure(
                implementation_->pending->request, engine::InputMethodStatus::cancelled));
        implementation_->pending = std::move(job);
    }
    cancellation.request_stop();
    implementation_->condition.notify_one();
    return future;
}
} // namespace owo::core
