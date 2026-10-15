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
        call.required_permissions = {"input.scheme"};
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
        if (response.status != InputMethodStatus::success)
            return failure(r, response.status, response.status == InputMethodStatus::cancelled ? std::string{} :
                response.status == InputMethodStatus::unavailable ? "Input scheme unavailable" : "Input scheme reported an error");
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
            active.inventory_sha256, active.publisher_certificate_sha256, "input.scheme");
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
    const std::filesystem::path& root, engine::InputMethodDescriptor descriptor, std::chrono::milliseconds timeout,
    std::string_view expected_version, std::string_view expected_inventory, std::string_view expected_publisher) {
    const auto validated = engine::validate_input_method_descriptor(descriptor);
    if (!validated.ok) return validated;
    if (timeout.count() <= 0 || timeout > std::chrono::seconds(5)) return {false, "Invalid scheme timeout"};
    const auto active = plugin::query_active_plugin_version(root, descriptor.plugin_id);
    if (!active.ok || active.manifest.runtime != "process") return {false, "Active process plugin required"};
    if ((!expected_version.empty() && active.manifest.version != expected_version) ||
        (!expected_inventory.empty() && active.inventory_sha256 != expected_inventory) ||
        (!expected_publisher.empty() && active.publisher_certificate_sha256 != expected_publisher))
        return {false, "Input scheme binding changed during discovery"};
    const auto grant = plugin::load_plugin_authorization(root, descriptor.plugin_id, active.manifest.version);
    if (!grant.ok || !plugin::is_plugin_permission_granted(grant.value, active.manifest,
        active.inventory_sha256, active.publisher_certificate_sha256, "input.scheme"))
        return {false, "Explicit input.scheme authorization required"};
    auto provider = std::make_shared<ProcessProvider>(root, descriptor, active, timeout);
    return registry.add(std::move(descriptor), std::move(provider));
}

InputMethodDiscovery discover_plugin_input_methods(const std::filesystem::path& root,
    std::string_view plugin_id, std::chrono::milliseconds timeout) {
    const auto reject = [](std::string diagnostic) {
        InputMethodDiscovery result;
        result.validation = {false, std::move(diagnostic)};
        return result;
    };
    if (timeout.count() <= 0 || timeout > std::chrono::seconds(5)) return reject("Invalid discovery timeout");
    const auto active = plugin::query_active_plugin_version(root, plugin_id);
    if (!active.ok || active.manifest.runtime != "process") return reject("Active process plugin required");
    const auto authorized = [&] {
        const auto current = plugin::query_active_plugin_version(root, plugin_id);
        if (!current.ok || current.installed_path != active.installed_path ||
            current.manifest.version != active.manifest.version || current.inventory_sha256 != active.inventory_sha256 ||
            current.publisher_certificate_sha256 != active.publisher_certificate_sha256) return false;
        const auto grant = plugin::load_plugin_authorization(root, plugin_id, current.manifest.version);
        return grant.ok && plugin::is_plugin_permission_granted(grant.value, current.manifest,
            current.inventory_sha256, current.publisher_certificate_sha256, "input.scheme");
    };
    if (!authorized()) return reject("Explicit input.scheme authorization required");
    const auto deadline = std::chrono::steady_clock::now() + timeout;
    auto launched = plugin::launch_active_plugin(root, plugin_id, timeout);
    if (!launched) return reject("Input scheme discovery process unavailable");
    const auto remaining = std::chrono::duration_cast<std::chrono::milliseconds>(deadline - std::chrono::steady_clock::now());
    if (remaining.count() <= 0) return reject("Input scheme discovery timed out");
    plugin::PluginInvokeRequest request;
    request.service = plugin::kInputMethodDescribeService;
    request.required_permissions = {"input.scheme"};
    request.user_initiated = true;
    request.timeout = remaining;
    const auto invoked = launched.session.invoke(std::move(request));
    if (!invoked.ok) return reject("Input scheme discovery failed");
    if (!authorized()) return reject("Input scheme authorization changed during discovery");
    InputMethodDiscovery result;
    result.validation = plugin::decode_input_method_descriptors(plugin_id, invoked.payload, result.descriptors);
    if (!result.validation.ok) return reject("Invalid input scheme descriptors");
    result.version = active.manifest.version;
    result.inventory = active.inventory_sha256;
    result.publisher = active.publisher_certificate_sha256;
    return result;
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

void InputMethodWorker::cancel() {
    std::stop_source cancellation;
    {
        std::lock_guard lock(implementation_->mutex);
        ++implementation_->generation;
        cancellation = implementation_->active;
        if (implementation_->pending) {
            implementation_->pending->promise.set_value(failure(
                implementation_->pending->request, engine::InputMethodStatus::cancelled));
            implementation_->pending.reset();
        }
    }
    cancellation.request_stop();
}

struct InputMethodSessionRouter::Impl {
    explicit Impl(std::filesystem::path value) : root(std::move(value)), worker(registry) {}
    std::filesystem::path root;
    engine::InputMethodRegistry registry;
    InputMethodWorker worker;
    std::mutex mutex;
    std::mutex control_mutex;
    std::uint64_t owner{};
    std::string session;
    engine::InputMethodDescriptor descriptor;
    std::string version;
    std::string inventory;
    std::string publisher;
    std::uint64_t last_request{};
    std::uint64_t generation{};
    std::uint64_t opening_owner{};
    std::string opening_session;
};
InputMethodSessionRouter::InputMethodSessionRouter(std::filesystem::path root)
    : implementation_(std::make_unique<Impl>(std::move(root))) {}
InputMethodSessionRouter::~InputMethodSessionRouter() = default;

engine::InputMethodValidation InputMethodSessionRouter::open(std::uint64_t owner, std::string session,
    std::string_view plugin_id, std::string_view scheme_id) {
    auto& state = *implementation_;
    std::lock_guard control(state.control_mutex);
    if (owner == 0 || session.empty() || session.size() > 128) return {false, "Invalid scheme session"};
    std::uint64_t generation{};
    {
        std::lock_guard lock(state.mutex);
        if ((state.owner != 0 && state.owner != owner) ||
            (state.opening_owner != 0 && state.opening_owner != owner))
            return {false, "Another owner has an active scheme session"};
        generation = ++state.generation;
        state.opening_owner = owner; state.opening_session = session;
    }
    const auto abandon = [&](engine::InputMethodValidation validation) {
        std::lock_guard lock(state.mutex);
        if (generation == state.generation) { state.opening_owner = 0; state.opening_session.clear(); }
        return validation;
    };
    auto discovery = discover_plugin_input_methods(state.root, plugin_id);
    if (!discovery.validation.ok) return abandon(discovery.validation);
    const auto found = std::find_if(discovery.descriptors.begin(), discovery.descriptors.end(),
        [&](const auto& d) { return d.id == scheme_id; });
    if (found == discovery.descriptors.end()) return abandon({false, "Scheme was not advertised by the plugin"});
    engine::InputMethodRequest empty;
    empty.request_id = 1; empty.session_id = session; empty.scheme_id = found->id;
    empty.page_size = (std::min)(std::size_t{7}, found->max_candidates);
    if (!engine::validate_input_method_request(*found, empty).ok) return abandon({false, "Invalid scheme session"});
    {
        std::lock_guard lock(state.mutex);
        if (generation != state.generation) return {false, "Scheme session changed during discovery"};
        state.opening_owner = 0; state.opening_session.clear();
        state.worker.cancel();
        if (state.owner != 0) state.registry.remove_plugin(state.descriptor.plugin_id);
        state.owner = 0; state.session.clear();
        const auto registered = register_plugin_input_method(state.registry, state.root, *found,
            std::chrono::milliseconds(1000), discovery.version, discovery.inventory, discovery.publisher);
        ++state.generation;
        if (!registered.ok) return registered;
        state.owner = owner; state.session = std::move(session); state.descriptor = *found; state.last_request = 0;
        state.version = discovery.version; state.inventory = discovery.inventory; state.publisher = discovery.publisher;
    }
    return {true, {}};
}

std::future<engine::InputMethodResponse> InputMethodSessionRouter::submit(std::uint64_t owner,
    engine::InputMethodRequest request, bool sensitive) {
    auto& state = *implementation_;
    std::lock_guard lock(state.mutex);
    const auto rejected = [&](engine::InputMethodStatus status, std::string diagnostic) {
        std::promise<engine::InputMethodResponse> promise;
        auto future = promise.get_future();
        promise.set_value(failure(request, status, std::move(diagnostic)));
        return future;
    };
    if (owner == 0 || state.owner != owner || state.session != request.session_id ||
        state.descriptor.id != request.scheme_id)
        return rejected(engine::InputMethodStatus::unavailable, "Input scheme session is not bound to this owner");
    if (sensitive) {
        state.worker.cancel();
        state.registry.remove_plugin(state.descriptor.plugin_id);
        state.owner = 0; state.session.clear(); ++state.generation;
        return rejected(engine::InputMethodStatus::error, "Sensitive input cannot be sent to an input scheme plugin");
    }
    const auto validation = engine::validate_input_method_request(state.descriptor, request);
    if (!validation.ok) return rejected(engine::InputMethodStatus::error, validation.diagnostic);
    if (request.request_id <= state.last_request)
        return rejected(engine::InputMethodStatus::error, "Input scheme request is stale");
    state.last_request = request.request_id;
    return state.worker.submit(std::move(request));
}

bool InputMethodSessionRouter::close(std::uint64_t owner, std::string_view session) {
    auto& state = *implementation_;
    std::lock_guard lock(state.mutex);
    const bool active = state.owner == owner && state.session == session;
    const bool opening = state.opening_owner == owner && state.opening_session == session;
    if (owner == 0 || (!active && !opening)) return false;
    if (active) {
        state.worker.cancel();
        state.registry.remove_plugin(state.descriptor.plugin_id);
        state.owner = 0; state.session.clear();
    }
    state.opening_owner = 0; state.opening_session.clear(); ++state.generation;
    return true;
}
std::optional<engine::InputMethodDescriptor> InputMethodSessionRouter::descriptor(
    std::uint64_t owner, std::string_view session) const {
    auto& state = *implementation_;
    engine::InputMethodDescriptor descriptor;
    std::string version, inventory, publisher;
    std::uint64_t generation{};
    {
        std::lock_guard lock(state.mutex);
        if (!owner || state.owner != owner || state.session != session) return std::nullopt;
        descriptor = state.descriptor; generation = state.generation;
        version = state.version; inventory = state.inventory; publisher = state.publisher;
    }
    // Revalidate when a queued result is published, not only while computing it.
    const auto active = plugin::query_active_plugin_version(state.root, descriptor.plugin_id);
    if (!active.ok || active.manifest.version != version || active.inventory_sha256 != inventory ||
        active.publisher_certificate_sha256 != publisher) return std::nullopt;
    const auto grant = plugin::load_plugin_authorization(state.root, descriptor.plugin_id, version);
    if (!grant.ok || !plugin::is_plugin_permission_granted(grant.value, active.manifest,
        inventory, publisher, "input.scheme")) return std::nullopt;
    std::lock_guard lock(state.mutex);
    if (state.generation != generation || state.owner != owner || state.session != session) return std::nullopt;
    return descriptor;
}
} // namespace owo::core
