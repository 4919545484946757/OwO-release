#include "owo/core/input_method_runtime.h"
#include "owo/plugin/plugin_authorization_store.h"
#include "owo/plugin/plugin_sandbox.h"
#include "owo/plugin/plugin_store.h"
#include "owo/plugin/input_method_protocol.h"
#include "owo/ipc/input_method_control.h"
#include "owo/ipc/input_method_server.h"
#include "owo/ipc/input_method_client.h"
#include "owo/tsf/input_method_session.h"
#include "owo/ipc/named_pipe.h"
#include "scheme.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>

#include <fstream>
#include <iostream>
#include <stdexcept>
#include <thread>
#include <atomic>

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
void pipe_tests(const std::filesystem::path& root, const engine::InputMethodDescriptor& d,
                const std::string& suffix) {
    using namespace std::chrono_literals;
    std::atomic<std::uint64_t> focus{1};
    std::atomic<std::uint32_t> observed_pid{};
    const auto name = LR"(\\.\pipe\OwO.Test.InputSchemes.)" + std::wstring(suffix.begin(), suffix.end());
    ipc::InputMethodPipeServer server(root, name, [&](std::uint32_t pid) {
        observed_pid = pid;
        return ipc::InputMethodFocus{focus.load(), focus.load() == 0};
    });
    require(server.started(), "experimental pipe started");
    ipc::InputMethodPipeServer collision(root, name);
    require(!collision.started(), "pipe name cannot be taken over");
    ipc::PersistentPipeClient client(name);
    std::uint64_t id{};
    const auto send = [&](ipc::InputMethodControlRequest r) {
        if (!r.request_id) r.request_id = ++id;
        auto exchanged = client.exchange(ipc::encode_input_method_control_request(r), 2s);
        require(static_cast<bool>(exchanged.status), "control pipe exchange");
        ipc::InputMethodControlResponse response;
        require(static_cast<bool>(ipc::decode_input_method_control_response(exchanged.response, response)) &&
            response.request_id == r.request_id, "control response identity");
        return response;
    };
    const auto wait = [&](std::uint64_t ticket) {
        const auto deadline = std::chrono::steady_clock::now() + 4s;
        for (;;) {
            ipc::InputMethodControlRequest poll;
            poll.action = ipc::InputMethodControlAction::poll; poll.ticket = ticket;
            auto result = send(poll);
            if (result.status != ipc::InputMethodControlStatus::pending) return result;
            require(std::chrono::steady_clock::now() < deadline, "control result deadline");
            std::this_thread::sleep_for(10ms);
        }
    };
    ipc::InputMethodControlRequest r;
    r.plugin_id = d.plugin_id;
    auto discovery = send(r);
    require(discovery.status == ipc::InputMethodControlStatus::pending, "discovery does not block transport");
    auto metadata = wait(discovery.ticket);
    std::vector<engine::InputMethodDescriptor> descriptions;
    require(metadata.status == ipc::InputMethodControlStatus::ready &&
        plugin::decode_input_method_descriptors(d.plugin_id, metadata.payload, descriptions).ok &&
        descriptions.size() == 1, "real pipe discovery");
    r.action = ipc::InputMethodControlAction::open; r.scheme_id = d.id; r.session_id = "pipe-session";
    const auto open = [&] {
        const auto result = send(r);
        require(result.status == ipc::InputMethodControlStatus::pending &&
            wait(result.ticket).status == ipc::InputMethodControlStatus::ready, "real pipe open");
    };
    open();
    require(observed_pid == GetCurrentProcessId(), "client PID obtained from actual pipe");
    engine::InputMethodRequest input;
    input.session_id = r.session_id; input.scheme_id = d.id; input.page_size = 2;
    const auto submit = [&](const char* text) {
        input.request_id = ++id; input.input = text; input.caret_byte = input.input.size();
        ipc::InputMethodControlRequest req;
        req.request_id = id; req.action = ipc::InputMethodControlAction::submit;
        req.payload = plugin::encode_input_method_request(d, input);
        return send(req);
    };
    auto first = submit("a");
    ipc::InputMethodControlRequest invalid;
    invalid.action = ipc::InputMethodControlAction::submit; invalid.payload = "malformed";
    require(send(invalid).status == ipc::InputMethodControlStatus::rejected,
        "invalid input does not replace active request");
    auto first_result = wait(first.ticket);
    engine::InputMethodResponse response;
    require(first_result.status == ipc::InputMethodControlStatus::ready &&
        plugin::decode_input_method_response(d, input, first_result.payload, response).ok &&
        response.candidates.front().text == "日", "real pipe candidate response");
    auto slow = submit("hang");
    auto newer = submit("ab");
    require(slow.status == ipc::InputMethodControlStatus::pending &&
        newer.status == ipc::InputMethodControlStatus::pending, "slow plugin leaves transport responsive");
    require(wait(slow.ticket).status == ipc::InputMethodControlStatus::rejected, "stale ticket rejected");
    auto latest = wait(newer.ticket);
    require(latest.status == ipc::InputMethodControlStatus::ready &&
        plugin::decode_input_method_response(d, input, latest.payload, response).ok &&
        response.candidates.front().text == "明", "latest input wins through pipe");
    ipc::InputMethodControlRequest replay; replay.request_id = id;
    replay.action = ipc::InputMethodControlAction::poll; replay.ticket = newer.ticket;
    require(send(replay).status == ipc::InputMethodControlStatus::rejected, "control replay rejected");
    auto pending = submit("slow");
    focus = 2;
    require(wait(pending.ticket).status == ipc::InputMethodControlStatus::rejected, "focus change discards result");
    require(submit("a").status == ipc::InputMethodControlStatus::rejected, "focus change requires reopen");
    open();
    ipc::InputMethodControlRequest sensitive;
    sensitive.action = ipc::InputMethodControlAction::poll; sensitive.ticket = 999; sensitive.sensitive_input = true;
    require(send(sensitive).status == ipc::InputMethodControlStatus::rejected &&
        submit("a").status == ipc::InputMethodControlStatus::rejected, "client can only tighten safety");
    focus = 0;
    require(send(r).status == ipc::InputMethodControlStatus::rejected, "unsafe host focus blocks open");
    focus = 3; open();
    pending = submit("slow");
    client.reset();
    require(wait(pending.ticket).status == ipc::InputMethodControlStatus::rejected, "reconnection cannot retrieve old result");
    require(submit("a").status == ipc::InputMethodControlStatus::rejected, "disconnection closes session");
    // Close while discovery is running; a late completion must never reopen the session.
    auto opening = send(r);
    require(opening.status == ipc::InputMethodControlStatus::pending, "pending open after reconnect");
    ipc::InputMethodControlRequest close;
    close.action = ipc::InputMethodControlAction::close; close.session_id = r.session_id;
    require(send(close).status == ipc::InputMethodControlStatus::ready, "close cancels pending control");
    std::this_thread::sleep_for(250ms);
    require(wait(opening.ticket).status == ipc::InputMethodControlStatus::rejected &&
        submit("a").status == ipc::InputMethodControlStatus::rejected, "late open cannot revive session");
    open();
    focus = 4;
    std::this_thread::sleep_for(100ms); // no client traffic: IO wait must still invalidate focus
    {
        ipc::PersistentPipeClient next_client(name);
        auto next_open = r; next_open.request_id = 1;
        const auto exchanged = next_client.exchange(ipc::encode_input_method_control_request(next_open), 2s);
        ipc::InputMethodControlResponse accepted;
        require(static_cast<bool>(exchanged.status) &&
            static_cast<bool>(ipc::decode_input_method_control_response(exchanged.response, accepted)) &&
            accepted.status == ipc::InputMethodControlStatus::pending,
            "idle previous client releases pipe to next foreground owner");
        auto next_close = close; next_close.request_id = 2;
        require(static_cast<bool>(next_client.exchange(ipc::encode_input_method_control_request(next_close), 2s).status),
            "next owner closes its pending open");
    }
    std::this_thread::sleep_for(200ms);
    require(submit("a").status == ipc::InputMethodControlStatus::rejected, "idle focus invalidates session");
    open();
    auto queued = submit("ab");
    std::this_thread::sleep_for(300ms);
    const auto active = plugin::query_active_plugin_version(root, d.plugin_id);
    const auto revoked = plugin::make_plugin_authorization(active.manifest, active.inventory_sha256,
        active.publisher_certificate_sha256, {});
    require(revoked.ok && plugin::save_plugin_authorization(root, revoked.value).ok, "revoke before publication");
    require(wait(queued.ticket).status == ipc::InputMethodControlStatus::rejected,
        "queued result rechecks authorization at publication");
    const auto restored = plugin::make_plugin_authorization(active.manifest, active.inventory_sha256,
        active.publisher_certificate_sha256, {"input.scheme"});
    require(restored.ok && plugin::save_plugin_authorization(root, restored.value).ok, "restore after publication test");
    // Also verify the production resolver: this console is not a foreground Edit/RichEdit input.
    ipc::InputMethodPipeServer native(root, name + L".native");
    require(native.started(), "native policy pipe started");
    ipc::PersistentPipeClient native_client(name + L".native");
    r.request_id = 1;
    const auto denied = native_client.exchange(ipc::encode_input_method_control_request(r), 2s);
    ipc::InputMethodControlResponse out;
    require(static_cast<bool>(denied.status) &&
        static_cast<bool>(ipc::decode_input_method_control_response(denied.response, out)) &&
        out.status == ipc::InputMethodControlStatus::rejected, "production policy fails closed for unknown input");
}
void async_client_tests(const std::filesystem::path& root, const engine::InputMethodDescriptor& d,
                        const std::string& suffix) {
    using namespace std::chrono_literals;
    const auto name = LR"(\\.\pipe\OwO.Test.InputSchemes.Async.)" + std::wstring(suffix.begin(), suffix.end());
    ipc::InputMethodPipeServer server(root, name, [](std::uint32_t) { return ipc::InputMethodFocus{1, false}; });
    require(server.started(), "async test pipe started");
    ipc::InputMethodAsyncClient client(name);
    const auto get_result = [](std::future<ipc::InputMethodClientResult> future) {
        require(future.wait_for(5s) == std::future_status::ready, "async client future deadline");
        return future.get();
    };
    auto discovered = get_result(client.discover(1, d.plugin_id));
    require(static_cast<bool>(discovered.status) && discovered.descriptors.size() == 1, "async discovery");
    tsf::InputMethodDocumentSession document;
    require(document.enter(1) && !document.bind(discovered, "discovery-only"),
        "discovery metadata cannot masquerade as opened session");
    require(document.enter(10), "trusted document generation");
    auto opened = get_result(client.open(10, d.plugin_id, d.id, "async-session", false));
    require(document.bind(opened, "async-session"), "async open binds document composition");
    auto& composition = document.composition();
    require(composition.insert("a"), "document raw encoding");
    require(!document.bind(opened, "async-session") && composition.input() == "a",
        "completed open cannot be applied twice to erase raw input");
    auto request = composition.compose_request(1, 0, 2);
    require(request.has_value(), "document request");
    auto candidates = get_result(client.submit(10, *request, false));
    require(candidates.response && candidates.response->request_id == 1 &&
        candidates.response->candidates.front().text == "日" && document.apply(candidates).accepted,
        "wire poll IDs map back to local composition IDs");
    auto selected = composition.select_request(2, "a");
    require(selected.has_value(), "document selection");
    auto acknowledgement = get_result(client.submit(10, *selected, false));
    auto prepared = document.apply(acknowledgement);
    require(prepared.commit && prepared.commit->text == "日" && composition.input() == "a" &&
        document.finish_commit(10, true) && composition.input().empty(), "async two-stage candidate commit");
    engine::InputMethodRequest input;
    input.request_id = 3; input.session_id = "async-session"; input.scheme_id = d.id;
    input.input = "hang"; input.caret_byte = 4; input.page_size = 2;
    const auto marker = root / "data" / d.plugin_id / "input-active.txt";
    std::filesystem::remove(marker);
    auto stale = client.submit(10, input, false);
    const auto deadline = std::chrono::steady_clock::now() + 2s;
    while (!std::filesystem::exists(marker) && std::chrono::steady_clock::now() < deadline)
        std::this_thread::sleep_for(5ms);
    require(std::filesystem::exists(marker), "async slow task actually entered plugin");
    input.request_id = 4; input.input = "ab"; input.caret_byte = 2;
    auto latest = client.submit(10, input, false);
    require(get_result(std::move(stale)).status.error == protocol::ErrorCode::cancelled,
        "new input cancels old async publication");
    auto value = get_result(std::move(latest));
    require(value.response && value.response->request_id == 4 && value.response->candidates.front().text == "明",
        "async latest input accepted");
    input.request_id = 5; input.input = "slow"; input.caret_byte = 4;
    auto valid = client.submit(10, input, false);
    auto invalid = input; invalid.request_id = 6; invalid.caret_byte = 999;
    require(get_result(client.submit(10, invalid, false)).status.error == protocol::ErrorCode::invalid_payload,
        "invalid async task rejected before supersession");
    require(get_result(std::move(valid)).response.has_value(), "invalid task leaves valid task intact");
    std::vector<std::future<ipc::InputMethodClientResult>> burst;
    for (std::uint64_t number = 6; number <= 25; ++number) {
        auto item = input; item.request_id = number; item.input = "ab"; item.caret_byte = 2;
        burst.push_back(client.submit(10, item, false));
    }
    for (std::size_t at = 0; at + 1 < burst.size(); ++at) {
        const auto item = get_result(std::move(burst[at]));
        require(static_cast<bool>(item.status) || item.status.error == protocol::ErrorCode::cancelled,
            "bounded burst resolves superseded futures");
    }
    auto burst_latest = get_result(std::move(burst.back()));
    require(burst_latest.response && burst_latest.response->request_id == 25 &&
        burst_latest.response->candidates.front().text == "明", "burst keeps latest task");
    require(!get_result(client.submit(99, input, false)).status, "wrong document cannot submit");
    input.request_id = 26; input.input = "hang"; input.caret_byte = 4;
    auto cancelled = client.submit(10, input, false);
    document.invalidate();
    require(static_cast<bool>(get_result(client.close()).status) &&
        get_result(std::move(cancelled)).status.error == protocol::ErrorCode::cancelled,
        "document cancellation closes background connection");
    require(!document.apply(candidates).accepted && !document.bind(opened, "async-session") &&
        !document.finish_commit(10, true), "completed old futures cannot reopen or commit after focus loss");
    require(document.enter(11) && !document.enter(10), "document generations cannot be reused");
    opened = get_result(client.open(11, d.plugin_id, d.id, "new-session", false));
    require(document.bind(opened, "new-session") && !document.apply(candidates).accepted,
        "new document rejects previous completed result");
    input.request_id = 1; input.session_id = "new-session"; input.input = "a"; input.caret_byte = 1;
    require(!get_result(client.submit(11, input)).status && !get_result(client.submit(11, input, false)).status,
        "default sensitive guard closes binding");
    require(static_cast<bool>(get_result(client.close()).status), "explicit close finishes");
    ipc::InputMethodAsyncClient missing(name + L".missing");
    require(!get_result(missing.open(1, d.plugin_id, d.id, "missing", false)).status,
        "unavailable Core returns bounded failure");
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
        std::filesystem::create_directories(staging / "assets");
        require(write(staging / "assets" / "dictionary-test.txt", "packaged-dictionary"), "dictionary asset fixture");
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
        pipe_tests(root, descriptor, suffix);
        async_client_tests(root, descriptor, suffix);
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
