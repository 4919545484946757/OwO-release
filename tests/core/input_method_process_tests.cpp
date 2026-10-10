#include "owo/core/input_method_runtime.h"
#include "owo/plugin/plugin_authorization_store.h"
#include "owo/plugin/plugin_sandbox.h"
#include "owo/plugin/plugin_store.h"
#include "scheme.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>

#include <fstream>
#include <iostream>
#include <stdexcept>
#include <thread>

using namespace owo;
namespace {
bool write(const std::filesystem::path& path, std::string_view text) {
    std::ofstream file(path, std::ios::binary);
    file.write(text.data(), static_cast<std::streamsize>(text.size()));
    return static_cast<bool>(file);
}
void require(bool ok, const char* name) { if (!ok) throw std::runtime_error(name); }
engine::InputMethodResponse get(std::future<engine::InputMethodResponse> future) {
    require(future.wait_for(std::chrono::seconds(8)) == std::future_status::ready, "worker future deadline");
    return future.get();
}
}
int main(int argc, char** argv) {
    if (argc != 3) return 1;
    const auto suffix = std::to_string(GetCurrentProcessId()) + "-" + std::to_string(GetTickCount64());
    const auto plugin_id = "owo.plugin.scheme-test-" + suffix;
    const auto root = std::filesystem::absolute(std::filesystem::path(std::string(argv[1]) + "." + suffix));
    int outcome = 0;
    bool owns_root = false;
    try {
        require(!std::filesystem::exists(root), "unique test directory");
        owns_root = true;
        require(plugin::initialize_plugin_store(root).ok, "initialize test store");
        const auto staging = root / "staging" / ".scheme";
        std::filesystem::create_directories(staging / "bin");
        std::filesystem::copy_file(argv[2], staging / "bin" / "probe.exe");
        require(write(staging / "config.schema.json", "{}") && write(staging / "manifest.json",
            "{\"id\":\"" + plugin_id + "\",\"name\":\"Scheme Probe\",\"version\":\"1.0.0\","
            "\"api_version\":1,\"runtime\":\"process\",\"entry\":\"bin/probe.exe\","
            "\"permissions\":[\"input.scheme\",\"input.context\"],\"network\":false,\"config_schema\":\"config.schema.json\"}"), "fixture files");
        require(plugin::publish_staged_plugin(root, staging, std::string(64, 'a'), std::string(64, 'b')).ok,
                "publish test plugin");
        const auto active = plugin::query_active_plugin_version(root, plugin_id);
        require(active.ok, "active binding");
        engine::InputMethodRegistry registry;
        const auto descriptor = example::table_descriptor(plugin_id);
        require(!core::register_plugin_input_method(registry, root, descriptor).ok, "ungranted registration rejected");
        require(!core::discover_plugin_input_methods(root, plugin_id).validation.ok, "ungranted discovery rejected");
        const auto old_grant = plugin::make_plugin_authorization(active.manifest, active.inventory_sha256,
            active.publisher_certificate_sha256, {"input.context"});
        require(old_grant.ok && plugin::save_plugin_authorization(root, old_grant.value).ok, "legacy context-only fixture grant");
        require(!core::register_plugin_input_method(registry, root, descriptor).ok &&
            !core::discover_plugin_input_methods(root, plugin_id).validation.ok, "context permission cannot imply scheme permission");
        const auto grant = plugin::make_plugin_authorization(active.manifest, active.inventory_sha256,
            active.publisher_certificate_sha256, {"input.scheme"});
        require(grant.ok && plugin::save_plugin_authorization(root, grant.value).ok, "explicit test authorization");
        const auto discovered = core::discover_plugin_input_methods(root, plugin_id);
        require(discovered.validation.ok && discovered.descriptors.size() == 1 &&
            discovered.descriptors.front().id == descriptor.id, "discover authorized process scheme");
        require(!core::register_plugin_input_method(registry, root, descriptor, std::chrono::milliseconds(1000),
            discovered.version, std::string(64, 'c')).ok, "stale discovery binding rejected");
        require(!core::register_plugin_input_method(registry, root, descriptor, std::chrono::milliseconds(1000),
            discovered.version, discovered.inventory, std::string(64, 'c')).ok, "stale publisher binding rejected");
        require(core::register_plugin_input_method(registry, root, descriptor, std::chrono::milliseconds(1500)).ok,
                "authorized process adapter registered");
        {
            core::InputMethodWorker worker(registry);
            engine::InputMethodRequest r;
            r.request_id = 1; r.session_id = "test-session"; r.scheme_id = descriptor.id;
            r.input = "a"; r.caret_byte = 1; r.page_size = 2;
            auto first = get(worker.submit(r));
            if (first.status != engine::InputMethodStatus::success) std::cerr << first.diagnostic << '\n';
            require(first.status == engine::InputMethodStatus::success && first.has_more &&
                first.candidates.size() == 2 && first.candidates.front().text == "日", "AppContainer table candidates");
            ++r.request_id; r.page = 1;
            auto second = get(worker.submit(r));
            require(second.candidates.size() == 1 && second.candidates.front().text == "明" && !second.has_more, "page identity and contents");
            ++r.request_id;
            r.action = engine::InputMethodAction::select; r.selected_candidate_id = "ab";
            require(get(worker.submit(r)).status == engine::InputMethodStatus::success, "selection forwarded");
            ++r.request_id; r.selected_candidate_id = "unissued";
            require(get(worker.submit(r)).status == engine::InputMethodStatus::error, "unknown candidate selection rejected");
            ++r.request_id; r.action = engine::InputMethodAction::cancel; r.selected_candidate_id.clear();
            require(get(worker.submit(r)).status == engine::InputMethodStatus::cancelled, "session cancellation action");
            r.action = engine::InputMethodAction::compose;
            r.page = 0;
            for (const auto code : {"bad", "crash", "hang"}) {
                ++r.request_id; r.input = code; r.caret_byte = r.input.size();
                const auto started = std::chrono::steady_clock::now();
                auto failed = get(worker.submit(r));
                require(failed.status == engine::InputMethodStatus::error && failed.candidates.empty(), "fault cannot produce selectable candidates");
                require(std::chrono::steady_clock::now() - started < std::chrono::seconds(5), "failure is bounded");
                require(r.input == code, "input preserved on error");
                ++r.request_id; r.input = "ab"; r.caret_byte = 2;
                auto recovered = get(worker.submit(r));
                require(recovered.status == engine::InputMethodStatus::success && recovered.candidates.front().text == "明", "recover after invalid response/crash/timeout");
            }
            const auto marker = root / "data" / plugin_id / "input-active.txt";
            std::filesystem::remove(marker);
            ++r.request_id; r.input = "hang"; r.caret_byte = 4;
            auto stale = worker.submit(r);
            const auto ready_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
            while (!std::filesystem::exists(marker) && std::chrono::steady_clock::now() < ready_deadline)
                std::this_thread::sleep_for(std::chrono::milliseconds(5));
            require(std::filesystem::exists(marker), "active IPC invocation marker");
            ++r.request_id; r.input = "ab"; r.caret_byte = 2;
            auto newest = worker.submit(r);
            require(get(std::move(stale)).status == engine::InputMethodStatus::cancelled, "new input cancels active IPC call");
            require(get(std::move(newest)).status == engine::InputMethodStatus::success, "latest input recovers after cancellation");
            const auto revoked = plugin::make_plugin_authorization(active.manifest, active.inventory_sha256,
                active.publisher_certificate_sha256, {});
            std::filesystem::remove(marker);
            ++r.request_id; r.input = "slow"; r.caret_byte = 4;
            auto computing = worker.submit(r);
            const auto revoke_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
            while (!std::filesystem::exists(marker) && std::chrono::steady_clock::now() < revoke_deadline)
                std::this_thread::sleep_for(std::chrono::milliseconds(5));
            require(std::filesystem::exists(marker), "permission revoked during actual IPC work");
            require(revoked.ok && plugin::save_plugin_authorization(root, revoked.value).ok, "revoke test permission");
            require(get(std::move(computing)).status == engine::InputMethodStatus::unavailable, "in-flight result discarded after revocation");
            ++r.request_id;
            require(get(worker.submit(r)).status == engine::InputMethodStatus::unavailable, "revocation blocks next input");
            require(plugin::save_plugin_authorization(root, grant.value).ok, "restore test authorization");
            require(plugin::deactivate_plugin(root, plugin_id, "1.0.0").ok, "disable test plugin");
            ++r.request_id;
            require(get(worker.submit(r)).status == engine::InputMethodStatus::unavailable, "disabled process result unavailable");
        }
        registry.remove_plugin(plugin_id);
        require(plugin::activate_installed_plugin_version(root, plugin_id, "1.0.0").ok, "reactivate fixture for session tests");
        {
            core::InputMethodSessionRouter router(root);
            engine::InputMethodRequest r;
            r.request_id = 1; r.session_id = "router-session"; r.scheme_id = descriptor.id;
            r.input = "ab"; r.caret_byte = 2;
            require(get(router.submit(100, r, false)).status == engine::InputMethodStatus::unavailable, "no implicit scheme session");
            require(!router.open(0, r.session_id, plugin_id, descriptor.id).ok, "owner identity required");
            require(!router.open(100, r.session_id, plugin_id, plugin_id + "/unknown").ok, "unadvertised scheme rejected");
            require(router.open(100, r.session_id, plugin_id, descriptor.id).ok, "explicit session binding");
            require(router.descriptor(100, r.session_id).has_value() &&
                router.descriptor(100, r.session_id)->id == descriptor.id, "bound descriptor available to owner");
            require(!router.descriptor(200, r.session_id) && !router.descriptor(100, "wrong"),
                "descriptor cannot cross owner or session");
            require(!router.open(200, "other", plugin_id, descriptor.id).ok, "another owner cannot replace session");
            require(get(router.submit(200, r, false)).status == engine::InputMethodStatus::unavailable, "cross-owner request rejected");
            auto wrong = r; wrong.session_id = "other";
            require(get(router.submit(100, wrong, false)).status == engine::InputMethodStatus::unavailable, "cross-session request rejected");
            require(get(router.submit(100, r, false)).status == engine::InputMethodStatus::success, "session routes to real process");
            require(get(router.submit(100, r, false)).status == engine::InputMethodStatus::error, "replayed request rejected");
            require(!router.close(200, r.session_id), "cross-owner close rejected");
            ++r.request_id;
            require(get(router.submit(100, r, true)).status == engine::InputMethodStatus::error, "sensitive input rejected");
            ++r.request_id;
            require(get(router.submit(100, r, false)).status == engine::InputMethodStatus::unavailable, "sensitive transition invalidates session");
            require(router.open(100, r.session_id, plugin_id, descriptor.id).ok, "explicit reopen after secure transition");
            require(router.close(100, r.session_id), "owner closes bound session");
            require(!router.descriptor(100, r.session_id), "closed descriptor unavailable");
            require(get(router.submit(100, r, false)).status == engine::InputMethodStatus::unavailable, "closed session has no output");
            const auto describe_marker = root / "data" / plugin_id / "describe-active.txt";
            std::filesystem::remove(describe_marker);
            auto opening = std::async(std::launch::async, [&] {
                return router.open(100, r.session_id, plugin_id, descriptor.id);
            });
            const auto open_deadline = std::chrono::steady_clock::now() + std::chrono::seconds(2);
            while (!std::filesystem::exists(describe_marker) && std::chrono::steady_clock::now() < open_deadline)
                std::this_thread::sleep_for(std::chrono::milliseconds(5));
            require(std::filesystem::exists(describe_marker), "discovery is in flight");
            require(router.close(100, r.session_id), "close cancels pending session open");
            require(!opening.get().ok && get(router.submit(100, r, false)).status == engine::InputMethodStatus::unavailable,
                "late discovery cannot reopen a closed session");
        }
    } catch (const std::exception& error) { std::cerr << error.what() << '\n'; outcome = 2; }
    // Only this test's unique absolute store and deterministic sandbox profile are removed.
    std::error_code error;
    if (!owns_root) return outcome;
    std::filesystem::remove_all(root, error);
    const auto profile = plugin::prepare_plugin_sandbox_profile(plugin_id);
    if (error || !profile.ok || !plugin::delete_plugin_sandbox_profile(profile.profile_name).ok) return 3;
    if (!outcome) std::cout << "Isolated input scheme process tests passed\n";
    return outcome;
}
