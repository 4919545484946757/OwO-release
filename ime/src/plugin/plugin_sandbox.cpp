#include "owo/plugin/plugin_sandbox.h"

#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#include <bcrypt.h>
#include <sddl.h>
#include <userenv.h>
#endif

#include <algorithm>
#include <array>
#include <cstddef>
#include <utility>
#include <vector>

namespace owo::plugin {
namespace {

constexpr std::wstring_view kProfilePrefix = L"OwO.Plugin.";

bool ascii_iequal(const wchar_t left, const wchar_t right) {
    const auto lower = [](const wchar_t value) {
        return value >= L'A' && value <= L'Z' ? value - L'A' + L'a' : value;
    };
    return lower(left) == lower(right);
}

bool owned_profile_name(const std::wstring_view value) {
    return value.size() == kProfilePrefix.size() + 32 &&
           std::equal(kProfilePrefix.begin(), kProfilePrefix.end(), value.begin(), ascii_iequal) &&
           std::all_of(value.begin() + static_cast<std::ptrdiff_t>(kProfilePrefix.size()),
                       value.end(), [](const wchar_t character) {
                           return (character >= L'0' && character <= L'9') ||
                                  (character >= L'a' && character <= L'f');
                       });
}

PluginSandboxProfileResult failure(std::string diagnostic) {
    return {false, false, {}, {}, {}, std::move(diagnostic)};
}

bool plugin_id_text(const std::string_view value) {
    if (value.size() < 3 || value.size() > 128 || value.front() == '.' ||
        value.back() == '.' || value.find('.') == std::string_view::npos) return false;
    bool previous_dot = false;
    for (const unsigned char byte : value) {
        const bool valid = (byte >= 'a' && byte <= 'z') || (byte >= '0' && byte <= '9') ||
                           byte == '-' || byte == '.';
        if (!valid || (byte == '.' && previous_dot)) return false;
        previous_dot = byte == '.';
    }
    return true;
}

#ifdef _WIN32
std::wstring derive_profile_name(const std::string_view plugin_id) {
    BCRYPT_ALG_HANDLE algorithm = nullptr;
    BCRYPT_HASH_HANDLE hash = nullptr;
    std::array<unsigned char, 32> digest{};
    if (BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) < 0 ||
        BCryptCreateHash(algorithm, &hash, nullptr, 0, nullptr, 0, 0) < 0 ||
        BCryptHashData(hash,
            const_cast<PUCHAR>(reinterpret_cast<const unsigned char*>(plugin_id.data())),
            static_cast<ULONG>(plugin_id.size()), 0) < 0 ||
        BCryptFinishHash(hash, digest.data(), static_cast<ULONG>(digest.size()), 0) < 0) {
        if (hash != nullptr) BCryptDestroyHash(hash);
        if (algorithm != nullptr) BCryptCloseAlgorithmProvider(algorithm, 0);
        return {};
    }
    BCryptDestroyHash(hash);
    BCryptCloseAlgorithmProvider(algorithm, 0);
    constexpr wchar_t hex[] = L"0123456789abcdef";
    std::wstring result(kProfilePrefix);
    for (std::size_t index = 0; index < 16; ++index) {
        result.push_back(hex[digest[index] >> 4U]);
        result.push_back(hex[digest[index] & 0x0fU]);
    }
    return result;
}

std::string hresult_error(const char* operation, const HRESULT result) {
    return std::string(operation) + " failed with HRESULT " +
           std::to_string(static_cast<unsigned long>(result));
}
#endif

}  // namespace

std::wstring plugin_sandbox_profile_name(const std::string_view plugin_id) {
#ifdef _WIN32
    return plugin_id_text(plugin_id) ? derive_profile_name(plugin_id) : std::wstring{};
#else
    static_cast<void>(plugin_id);
    return {};
#endif
}

PluginSandboxProfileResult prepare_plugin_sandbox_profile(const std::string_view plugin_id) {
#ifdef _WIN32
    if (!plugin_id_text(plugin_id)) return failure("invalid plugin id for sandbox profile");
    const auto name = plugin_sandbox_profile_name(plugin_id);
    if (name.empty()) return failure("cannot derive deterministic sandbox profile name");
    const std::wstring display_name = L"OwO Plugin Sandbox";
    const std::wstring description = L"Zero-capability sandbox for one OwO process plugin";
    PSID sid = nullptr;
    bool created = false;
    auto status = CreateAppContainerProfile(name.c_str(), display_name.c_str(),
                                            description.c_str(), nullptr, 0, &sid);
    if (status == HRESULT_FROM_WIN32(ERROR_ALREADY_EXISTS)) {
        status = DeriveAppContainerSidFromAppContainerName(name.c_str(), &sid);
    } else if (SUCCEEDED(status)) {
        created = true;
    }
    if (FAILED(status) || sid == nullptr)
        return failure(hresult_error("AppContainer profile preparation", status));
    LPWSTR sid_text = nullptr;
    if (!ConvertSidToStringSidW(sid, &sid_text) || sid_text == nullptr) {
        FreeSid(sid);
        if (created) DeleteAppContainerProfile(name.c_str());
        return failure("cannot format AppContainer SID");
    }
    PWSTR directory = nullptr;
    status = GetAppContainerFolderPath(sid_text, &directory);
    if (FAILED(status) || directory == nullptr) {
        LocalFree(sid_text);
        FreeSid(sid);
        if (created) DeleteAppContainerProfile(name.c_str());
        return failure(hresult_error("GetAppContainerFolderPath", status));
    }
    PluginSandboxProfileResult result{true, created, name, sid_text,
                                       std::filesystem::path(directory), {}};
    CoTaskMemFree(directory);
    LocalFree(sid_text);
    FreeSid(sid);
    return result;
#else
    static_cast<void>(plugin_id);
    return failure("plugin sandbox profiles are currently available on Windows only");
#endif
}

PluginSandboxProfileResult delete_plugin_sandbox_profile(const std::wstring_view profile_name) {
#ifdef _WIN32
    if (!owned_profile_name(profile_name))
        return failure("refusing to delete an unrecognized AppContainer profile name");
    const std::wstring owned_name(profile_name);
    const auto status = DeleteAppContainerProfile(owned_name.c_str());
    if (status == HRESULT_FROM_WIN32(ERROR_NOT_FOUND))
        return {true, false, owned_name, {}, {}, {}};
    if (FAILED(status)) return failure(hresult_error("DeleteAppContainerProfile", status));
    return {true, false, owned_name, {}, {}, {}};
#else
    static_cast<void>(profile_name);
    return failure("plugin sandbox profiles are currently available on Windows only");
#endif
}

}  // namespace owo::plugin
