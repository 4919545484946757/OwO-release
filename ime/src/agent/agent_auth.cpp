#include "owo/agent/agent_auth.h"
#include "owo/agent/agent_protocol.h"
#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#include <aclapi.h>
#include <algorithm>
#include <array>
#include <bcrypt.h>
#include <cstring>
#include <vector>
#include <wincrypt.h>
namespace {
struct Handle {
    HANDLE value{INVALID_HANDLE_VALUE};
    ~Handle() {
        if (value != INVALID_HANDLE_VALUE && value)
            CloseHandle(value);
    }
};
struct LocalMemory {
    void *value{};
    ~LocalMemory() {
        if (value)
            LocalFree(value);
    }
};
struct Plaintext {
    DATA_BLOB value{};
    ~Plaintext() {
        if (value.pbData) {
            SecureZeroMemory(value.pbData, value.cbData);
            LocalFree(value.pbData);
        }
    }
};
struct Algorithm {
    BCRYPT_ALG_HANDLE value{};
    ~Algorithm() {
        if (value)
            BCryptCloseAlgorithmProvider(value, 0);
    }
};
bool private_credential(HANDLE file) {
    PSID owner{};
    PACL acl{};
    PSECURITY_DESCRIPTOR descriptor{};
    if (GetSecurityInfo(file, SE_FILE_OBJECT,
                        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, &owner, nullptr,
                        &acl, nullptr, &descriptor) != ERROR_SUCCESS)
        return false;
    LocalMemory memory{descriptor};
    SECURITY_DESCRIPTOR_CONTROL control{};
    DWORD revision{};
    if (!GetSecurityDescriptorControl(descriptor, &control, &revision) ||
        !(control & SE_DACL_PROTECTED) || !acl || acl->AceCount != 1)
        return false;
    Handle token;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token.value))
        return false;
    DWORD size{};
    GetTokenInformation(token.value, TokenUser, nullptr, 0, &size);
    if (size == 0 || size > 65536)
        return false;
    std::vector<unsigned char> storage(size);
    if (!GetTokenInformation(token.value, TokenUser, storage.data(), size, &size))
        return false;
    const auto user = reinterpret_cast<TOKEN_USER *>(storage.data());
    if (!EqualSid(owner, user->User.Sid))
        return false;
    void *raw{};
    if (!GetAce(acl, 0, &raw))
        return false;
    const auto ace = static_cast<ACCESS_ALLOWED_ACE *>(raw);
    return ace->Header.AceType == ACCESS_ALLOWED_ACE_TYPE &&
           EqualSid(const_cast<DWORD *>(&ace->SidStart), user->User.Sid) &&
           (ace->Mask & FILE_ALL_ACCESS) == FILE_ALL_ACCESS;
}
void append_u64(std::string &out, std::uint64_t value) {
    for (unsigned n = 0; n < 8; ++n)
        out.push_back(static_cast<char>((value >> (8 * n)) & 255));
}
} // namespace
#endif
namespace owo::agent {
AuthenticatedRequest authenticate_agent_request(const std::string_view request,
                                                const std::filesystem::path &credential_file) {
#ifdef _WIN32
    constexpr std::size_t header = 108;
    if (request.empty() || request.size() > kMaximumAgentPayloadBytes - header ||
        !credential_file.is_absolute())
        return {};
    Handle file{CreateFileW(credential_file.c_str(), GENERIC_READ | READ_CONTROL, FILE_SHARE_READ,
                            nullptr, OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT, nullptr)};
    if (file.value == INVALID_HANDLE_VALUE || !private_credential(file.value))
        return {};
    BY_HANDLE_FILE_INFORMATION info{};
    if (!GetFileInformationByHandle(file.value, &info) ||
        (info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT | FILE_ATTRIBUTE_DIRECTORY)) ||
        info.nFileSizeHigh || info.nFileSizeLow <= 44 || info.nFileSizeLow > 44 + 16 * 1024)
        return {};
    std::vector<unsigned char> bytes(info.nFileSizeLow);
    DWORD read{};
    if (!ReadFile(file.value, bytes.data(), static_cast<DWORD>(bytes.size()), &read, nullptr) ||
        read != bytes.size() || std::memcmp(bytes.data(), "OwOKEY01", 8) != 0)
        return {};
    std::uint32_t size{};
    for (unsigned n = 0; n < 4; ++n)
        size |= static_cast<std::uint32_t>(bytes[40 + n]) << (8 * n);
    if (size != bytes.size() - 44 ||
        std::all_of(bytes.begin() + 8, bytes.begin() + 40, [](auto b) { return b == 0; }))
        return {};
    constexpr char purpose[] = "OwO/IME/pairing-key/v1";
    std::vector<unsigned char> entropy(purpose, purpose + sizeof(purpose));
    entropy.insert(entropy.end(), bytes.begin() + 8, bytes.begin() + 40);
    DATA_BLOB input{size, bytes.data() + 44};
    DATA_BLOB extra{static_cast<DWORD>(entropy.size()), entropy.data()};
    Plaintext key;
    if (!CryptUnprotectData(&input, nullptr, &extra, nullptr, nullptr, CRYPTPROTECT_UI_FORBIDDEN,
                            &key.value) ||
        key.value.cbData != 32 ||
        std::all_of(key.value.pbData, key.value.pbData + 32, [](auto b) { return b == 0; }))
        return {};
    std::array<unsigned char, 32> nonce{};
    if (BCryptGenRandom(nullptr, nonce.data(), static_cast<ULONG>(nonce.size()),
                        BCRYPT_USE_SYSTEM_PREFERRED_RNG) < 0 ||
        std::all_of(nonce.begin(), nonce.end(), [](auto b) { return b == 0; }))
        return {};
    constexpr char domain[] = "OwO/IME/pipe-request/v1";
    std::string signed_bytes(domain, sizeof(domain));
    signed_bytes.append(reinterpret_cast<const char *>(bytes.data() + 8), 32);
    signed_bytes.append(reinterpret_cast<const char *>(nonce.data()), nonce.size());
    append_u64(signed_bytes, request.size());
    signed_bytes.append(request);
    Algorithm algorithm;
    if (BCryptOpenAlgorithmProvider(&algorithm.value, BCRYPT_SHA256_ALGORITHM, nullptr,
                                    BCRYPT_ALG_HANDLE_HMAC_FLAG) < 0)
        return {};
    std::array<unsigned char, 32> tag{};
    if (BCryptHash(algorithm.value, key.value.pbData, 32,
                   reinterpret_cast<PUCHAR>(signed_bytes.data()),
                   static_cast<ULONG>(signed_bytes.size()), tag.data(),
                   static_cast<ULONG>(tag.size())) < 0)
        return {};
    std::string out = "OwOPRF01";
    out.append(reinterpret_cast<const char *>(bytes.data() + 8), 32);
    out.append(reinterpret_cast<const char *>(nonce.data()), nonce.size());
    out.append(reinterpret_cast<const char *>(tag.data()), tag.size());
    for (unsigned n = 0; n < 4; ++n)
        out.push_back(static_cast<char>((request.size() >> (8 * n)) & 255));
    out.append(request);
    return {true, std::move(out)};
#else
    static_cast<void>(request);
    static_cast<void>(credential_file);
    return {};
#endif
}
} // namespace owo::agent
