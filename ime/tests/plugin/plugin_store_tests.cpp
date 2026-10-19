#include "owo/plugin/plugin_authorization_store.h"
#include "owo/plugin/plugin_store.h"

#include <Windows.h>

#include <filesystem>
#include <fstream>
#include <iostream>
#include <iterator>
#include <string>

namespace {

std::string manifest_json(const std::string& plugin_id, const std::string& version) {
    return "{\"id\":\"" + plugin_id + "\",\"name\":\"Example\",\"version\":\"" + version +
           "\",\"api_version\":1,\"runtime\":\"process\",\"entry\":\"bin/example.exe\","
           "\"permissions\":[\"input.context\"],\"network\":false,"
           "\"config_schema\":\"config.schema.json\"}";
}

bool write_file(const std::filesystem::path& path, const std::string& bytes) {
    std::ofstream output(path, std::ios::binary | std::ios::trunc);
    output.write(bytes.data(), static_cast<std::streamsize>(bytes.size()));
    return static_cast<bool>(output);
}

bool remove_test_root(const std::filesystem::path& root) {
    for (unsigned attempt = 0; attempt < 10; ++attempt) {
        std::error_code error;
        std::filesystem::remove_all(root, error);
        error.clear();
        if (!std::filesystem::exists(root, error) && !error) return true;
        Sleep(25);
    }
    return false;
}

bool create_staging(const std::filesystem::path& root, const std::string& name,
                    const std::string& plugin_id, const std::string& version) {
    const auto staging = root / "staging" / name;
    std::error_code error;
    std::filesystem::create_directories(staging / "bin", error);
    return !error && write_file(staging / "manifest.json", manifest_json(plugin_id, version)) &&
           write_file(staging / "config.schema.json", "{}") &&
           write_file(staging / "bin" / "example.exe", "MZ");
}

std::string read_file(const std::filesystem::path& path) {
    std::ifstream input(path, std::ios::binary);
    return {std::istreambuf_iterator<char>(input), std::istreambuf_iterator<char>()};
}

bool has_item(const owo::plugin::PluginRecoveryScanResult& scan,
              const owo::plugin::PluginRecoveryKind kind,
              const std::string& version = {}) {
    for (const auto& item : scan.items) {
        if (item.kind == kind && (version.empty() || item.version == version)) return true;
    }
    return false;
}

const owo::plugin::PluginRecoveryItem* find_item(
    const owo::plugin::PluginRecoveryScanResult& scan,
    const owo::plugin::PluginRecoveryKind kind,
    const std::string& version = {}) {
    for (const auto& item : scan.items) {
        if (item.kind == kind && (version.empty() || item.version == version)) return &item;
    }
    return nullptr;
}

}  // namespace

int run_test(const int argc, char** argv) {
    if (argc != 2) return 1;
    const auto nonce = std::to_string(GetCurrentProcessId()) + "-" +
                       std::to_string(GetTickCount64());
    const std::filesystem::path requested_root(argv[1]);
    const auto root = requested_root.parent_path() /
        (requested_root.filename().string() + "-" + nonce);
    const auto plugin_id = "owo.plugin.store-test-" + nonce;
    std::error_code error;
    if (!remove_test_root(root)) return 2;
    const auto empty_scan = owo::plugin::scan_plugin_store_recovery(root);
    if (!empty_scan.ok || !empty_scan.items.empty() || std::filesystem::exists(root)) return 15;
    if (owo::plugin::scan_plugin_store_recovery("relative-plugin-store").ok) return 23;
    const auto initialized = owo::plugin::initialize_plugin_store(root);
    if (!initialized.ok || !std::filesystem::is_directory(root / "versions") ||
        !std::filesystem::is_directory(root / "authorizations") ||
        !std::filesystem::is_directory(root / "data") ||
        !std::filesystem::is_directory(root / "staging")) return 3;
    if (!create_staging(root, ".stage-v1", plugin_id, "1.0.0")) return 4;
    const auto first = owo::plugin::publish_staged_plugin(
        root, root / "staging" / ".stage-v1", std::string(64, 'a'), std::string(64, 'b'));
    if (!first.ok || !first.version_published || !first.activated ||
        !first.previous_version.empty() ||
        first.manifest.version != "1.0.0" || !std::filesystem::is_directory(first.installed_path) ||
        std::filesystem::exists(root / "staging" / ".stage-v1")) return 5;
    const auto binding = owo::plugin::query_installed_plugin_version(
        root, plugin_id, "1.0.0");
    if (!binding.ok || binding.inventory_sha256 != std::string(64, 'a') ||
        binding.publisher_certificate_sha256 != std::string(64, 'b')) return 24;
    const auto active_binding = owo::plugin::query_active_plugin_version(
        root, plugin_id);
    if (!active_binding.ok || active_binding.manifest.version != "1.0.0" ||
        active_binding.installed_path != binding.installed_path ||
        active_binding.inventory_sha256 != binding.inventory_sha256 ||
        active_binding.publisher_certificate_sha256 !=
            binding.publisher_certificate_sha256) return 31;
    const auto authorization = owo::plugin::make_plugin_authorization(
        first.manifest, binding.inventory_sha256, binding.publisher_certificate_sha256,
        {"input.context"});
    if (!authorization.ok) return 25;
    const auto saved_authorization = owo::plugin::save_plugin_authorization(root, authorization.value);
    const auto loaded_authorization = owo::plugin::load_plugin_authorization(
        root, plugin_id, "1.0.0");
    if (!saved_authorization.ok || !loaded_authorization.ok ||
        !owo::plugin::is_plugin_permission_granted(
            loaded_authorization.value, first.manifest, binding.inventory_sha256,
            binding.publisher_certificate_sha256, "input.context")) return 26;
    auto forged_authorization = authorization.value;
    forged_authorization.inventory_sha256.assign(64, 'f');
    if (owo::plugin::save_plugin_authorization(root, forged_authorization).ok) return 27;
    forged_authorization = authorization.value;
    forged_authorization.schema_version = owo::plugin::kPluginAuthorizationSchemaVersion + 1;
    if (owo::plugin::save_plugin_authorization(root, forged_authorization).ok) return 30;
    const auto active = root / "active" / (plugin_id + ".record");
    if (read_file(active).find("version=1.0.0\n") == std::string::npos) return 6;
    const auto data_marker = root / "data" / plugin_id / "marker.txt";
    if (!write_file(data_marker, "persistent")) return 7;

    if (!create_staging(root, ".stage-v2", plugin_id, "2.0.0")) return 8;
    const auto second = owo::plugin::publish_staged_plugin(
        root, root / "staging" / ".stage-v2", std::string(64, 'c'), std::string(64, 'd'));
    if (!second.ok || !second.version_published || !second.activated ||
        second.previous_version != "1.0.0" ||
        !std::filesystem::is_directory(root / "versions" / plugin_id / "1.0.0") ||
        !std::filesystem::is_directory(root / "versions" / plugin_id / "2.0.0") ||
        read_file(data_marker) != "persistent") {
        std::cerr << "second publish failed: " << second.diagnostic
                  << ", published=" << second.version_published
                  << ", activated=" << second.activated
                  << ", previous=" << second.previous_version
                  << ", v1=" << std::filesystem::is_directory(
                         root / "versions" / plugin_id / "1.0.0")
                  << ", v2=" << std::filesystem::is_directory(
                         root / "versions" / plugin_id / "2.0.0")
                  << ", data=" << read_file(data_marker) << '\n';
        return 9;
    }
    if (owo::plugin::load_plugin_authorization(
            root, plugin_id, "2.0.0").ok) return 28;
    const auto rollback = owo::plugin::activate_installed_plugin_version(
        root, plugin_id, "1.0.0");
    if (!rollback.ok || !rollback.version_published || !rollback.activated ||
        rollback.previous_version != "2.0.0" ||
        read_file(active).find("version=1.0.0\n") == std::string::npos) return 10;
    const auto listed = owo::plugin::list_installed_plugins(root);
    if (!listed.ok || listed.versions.size() != 2 || !listed.versions[0].active ||
        listed.versions[1].active) return 32;
    if (owo::plugin::deactivate_plugin(root, plugin_id, "2.0.0").ok ||
        !owo::plugin::query_active_plugin_version(root, plugin_id).ok) return 33;
    const auto deactivated = owo::plugin::deactivate_plugin(
        root, plugin_id, "1.0.0");
    if (!deactivated.ok || std::filesystem::exists(active) ||
        read_file(data_marker) != "persistent" ||
        !owo::plugin::load_plugin_authorization(root, plugin_id, "1.0.0").ok)
        return 34;
    const auto disabled_list = owo::plugin::list_installed_plugins(root);
    if (!disabled_list.ok || disabled_list.versions.size() != 2 ||
        disabled_list.versions[0].active || disabled_list.versions[1].active) return 35;
    const auto reenabled = owo::plugin::activate_installed_plugin_version(
        root, plugin_id, "1.0.0");
    if (!reenabled.ok || !std::filesystem::exists(active)) return 36;
    if (owo::plugin::uninstall_plugin_version(
            root, plugin_id, "1.0.0").ok) {
        std::cerr << "active version unexpectedly uninstalled\n";
        return 40;
    }

    if (!create_staging(root, ".stage-duplicate", plugin_id, "1.0.0")) return 11;
    if (owo::plugin::publish_staged_plugin(
            root, root / "staging" / ".stage-duplicate",
            std::string(64, 'e'), std::string(64, 'f')).ok) return 12;
    if (owo::plugin::activate_installed_plugin_version(
            root, "../escape", "1.0.0").ok) return 13;
    std::filesystem::remove_all(root / "staging" / ".stage-duplicate", error);
    if (error) return 16;
    const auto retained = root / "staging" / ".install-recovery";
    std::filesystem::create_directories(retained, error);
    if (error || !write_file(retained / "partial.bin", "partial")) return 17;
    const auto retained_uninstall = root / "staging" / ".uninstall-recovery";
    std::filesystem::create_directories(retained_uninstall, error);
    if (error || !write_file(retained_uninstall / "detached.bin", "detached")) return 41;
    const auto orphaned = root / "versions" / plugin_id / "3.0.0";
    std::filesystem::create_directories(orphaned / "bin", error);
    if (error || !write_file(orphaned / "manifest.json", manifest_json(plugin_id, "3.0.0")) ||
        !write_file(orphaned / "config.schema.json", "{}") ||
        !write_file(orphaned / "bin" / "example.exe", "MZ")) return 18;
    if (!write_file(root / "records" / plugin_id / "9.0.0.record", "invalid\n"))
        return 19;
    const auto recovery = owo::plugin::scan_plugin_store_recovery(root);
    if (!recovery.ok || recovery.items.size() != 5 ||
        !has_item(recovery, owo::plugin::PluginRecoveryKind::retained_staging) ||
        !has_item(recovery, owo::plugin::PluginRecoveryKind::retained_uninstall) ||
        !has_item(recovery, owo::plugin::PluginRecoveryKind::orphaned_version, "3.0.0") ||
        !has_item(recovery, owo::plugin::PluginRecoveryKind::orphaned_record) ||
        !has_item(recovery, owo::plugin::PluginRecoveryKind::inactive_version, "2.0.0") ||
        !std::filesystem::exists(retained) || !std::filesystem::exists(orphaned)) return 20;
    const auto* retained_item = find_item(
        recovery, owo::plugin::PluginRecoveryKind::retained_staging);
    const auto* retained_uninstall_item = find_item(
        recovery, owo::plugin::PluginRecoveryKind::retained_uninstall);
    const auto* orphaned_version_item = find_item(
        recovery, owo::plugin::PluginRecoveryKind::orphaned_version, "3.0.0");
    const auto* orphaned_record_item = find_item(
        recovery, owo::plugin::PluginRecoveryKind::orphaned_record);
    const auto* inactive_item = find_item(
        recovery, owo::plugin::PluginRecoveryKind::inactive_version, "2.0.0");
    if (!retained_item || !retained_uninstall_item || !orphaned_version_item ||
        !orphaned_record_item || !inactive_item ||
        !owo::plugin::cleanup_plugin_recovery_item(root, *retained_item).ok ||
        !owo::plugin::cleanup_plugin_recovery_item(root, *retained_uninstall_item).ok ||
        !owo::plugin::cleanup_plugin_recovery_item(root, *orphaned_version_item).ok ||
        !owo::plugin::cleanup_plugin_recovery_item(root, *orphaned_record_item).ok ||
        owo::plugin::cleanup_plugin_recovery_item(root, *inactive_item).ok ||
        std::filesystem::exists(retained) || std::filesystem::exists(retained_uninstall) ||
        std::filesystem::exists(orphaned) ||
        std::filesystem::exists(root / "records" / plugin_id / "9.0.0.record"))
        return 37;
    if (owo::plugin::cleanup_plugin_recovery_item(root, *retained_item).ok) return 38;
    if (!write_file(active, "invalid\n")) return 21;
    if (!write_file(saved_authorization.record_path, "invalid\n")) return 29;
    const auto damaged = owo::plugin::scan_plugin_store_recovery(root);
    if (!damaged.ok ||
        !has_item(damaged, owo::plugin::PluginRecoveryKind::invalid_active_record) ||
        !has_item(damaged, owo::plugin::PluginRecoveryKind::orphaned_authorization) ||
        !has_item(damaged, owo::plugin::PluginRecoveryKind::inactive_version, "1.0.0")) return 22;
    const auto* invalid_active = find_item(
        damaged, owo::plugin::PluginRecoveryKind::invalid_active_record);
    const auto* invalid_authorization = find_item(
        damaged, owo::plugin::PluginRecoveryKind::orphaned_authorization);
    if (!invalid_active || !invalid_authorization ||
        !owo::plugin::cleanup_plugin_recovery_item(root, *invalid_active).ok ||
        !owo::plugin::cleanup_plugin_recovery_item(root, *invalid_authorization).ok ||
        std::filesystem::exists(active) || std::filesystem::exists(saved_authorization.record_path))
        return 39;
    const auto removed_v2 = owo::plugin::uninstall_plugin_version(
        root, plugin_id, "2.0.0");
    if (!removed_v2.ok || !removed_v2.version_removed ||
        !removed_v2.authorization_removed || removed_v2.last_version ||
        removed_v2.sandbox_profile_removed || !removed_v2.data_preserved ||
        std::filesystem::exists(root / "versions" / plugin_id / "2.0.0") ||
        std::filesystem::exists(root / "records" / plugin_id / "2.0.0.record") ||
        read_file(data_marker) != "persistent") {
        std::cerr << "v2 uninstall failed: " << removed_v2.diagnostic << '\n';
        return 42;
    }
    const auto removed_v1 = owo::plugin::uninstall_plugin_version(
        root, plugin_id, "1.0.0");
    if (!removed_v1.ok || !removed_v1.version_removed || !removed_v1.last_version ||
        !removed_v1.sandbox_profile_removed || !removed_v1.data_preserved ||
        std::filesystem::exists(root / "versions" / plugin_id / "1.0.0") ||
        std::filesystem::exists(root / "records" / plugin_id / "1.0.0.record") ||
        read_file(data_marker) != "persistent") {
        std::cerr << "v1 uninstall failed: " << removed_v1.diagnostic << '\n';
        return 43;
    }
    const auto after_uninstall = owo::plugin::scan_plugin_store_recovery(root);
    if (!after_uninstall.ok || !after_uninstall.items.empty()) {
        std::cerr << "post-uninstall recovery scan failed: " << after_uninstall.diagnostic
                  << ", items=" << after_uninstall.items.size() << '\n';
        return 44;
    }
    if (!remove_test_root(root)) return 14;
    return 0;
}

int main(const int argc, char** argv) {
    const auto outcome = run_test(argc, argv);
    if (outcome != 0) std::cerr << "plugin store test failed at check " << outcome << '\n';
    return outcome;
}
