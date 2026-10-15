#include "owo/plugin/package_signature.h"

#include "owo/plugin/package_archive.h"

#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#define CERT_CHAIN_PARA_HAS_EXTRA_FIELDS
#define CRYPT_VERIFY_MESSAGE_PARA_HAS_EXTRA_FIELDS
#include <Windows.h>
#include <wincrypt.h>
#endif

#include <algorithm>
#include <array>
#include <charconv>
#include <cctype>
#include <limits>
#include <map>
#include <variant>

namespace owo::plugin {
namespace {

constexpr std::size_t kMaximumSignatureJsonBytes = 32U * 1024U;
constexpr std::size_t kMaximumCmsBytes = 24U * 1024U;
using Value = std::variant<std::string, std::uint64_t>;

class Parser final {
public:
    explicit Parser(const std::string_view input) : input_(input) {}

    bool object(std::map<std::string, Value>& fields) {
        space();
        if (!take('{')) return fail("signature metadata must be a JSON object");
        space();
        if (take('}')) return finish();
        while (true) {
            std::string key;
            if (!string(key)) return false;
            space();
            if (!take(':')) return fail("expected ':' after field name");
            space();
            Value value;
            if (key == "schema_version") {
                std::uint64_t number{};
                if (!unsigned_integer(number)) return false;
                value = number;
            } else if (key == "inventory_sha256" || key == "format" || key == "signature_base64") {
                std::string text;
                if (!string(text)) return false;
                value = std::move(text);
            } else {
                return fail("unknown field: " + key);
            }
            if (!fields.emplace(key, std::move(value)).second) return fail("duplicate field: " + key);
            space();
            if (take('}')) return finish();
            if (!take(',')) return fail("expected ',' or '}'");
            space();
        }
    }

    const std::string& error() const { return error_; }

private:
    void space() {
        while (offset_ < input_.size() &&
               (input_[offset_] == ' ' || input_[offset_] == '\t' ||
                input_[offset_] == '\r' || input_[offset_] == '\n')) ++offset_;
    }

    bool take(const char expected) {
        if (offset_ >= input_.size() || input_[offset_] != expected) return false;
        ++offset_;
        return true;
    }

    bool finish() {
        space();
        return offset_ == input_.size() || fail("trailing data after signature metadata");
    }

    bool string(std::string& output) {
        if (!take('"')) return fail("expected JSON string");
        while (offset_ < input_.size()) {
            const auto byte = static_cast<unsigned char>(input_[offset_++]);
            if (byte == '"') return true;
            if (byte < 0x20U || byte >= 0x80U) return fail("signature strings must be printable ASCII");
            if (byte == '\\') {
                if (offset_ >= input_.size()) return fail("truncated JSON escape");
                const auto escaped = input_[offset_++];
                if (escaped != '"' && escaped != '\\' && escaped != '/')
                    return fail("unsupported JSON escape");
                output.push_back(escaped);
            } else {
                output.push_back(static_cast<char>(byte));
            }
        }
        return fail("unterminated JSON string");
    }

    bool unsigned_integer(std::uint64_t& output) {
        const auto* begin = input_.data() + offset_;
        const auto* end = input_.data() + input_.size();
        const auto parsed = std::from_chars(begin, end, output);
        if (parsed.ec != std::errc{} || parsed.ptr == begin) return fail("expected unsigned integer");
        offset_ += static_cast<std::size_t>(parsed.ptr - begin);
        return true;
    }

    bool fail(std::string message) {
        if (error_.empty()) error_ = std::move(message) + " at byte " + std::to_string(offset_);
        return false;
    }

    std::string_view input_;
    std::size_t offset_{};
    std::string error_;
};

bool sha256_text(const std::string_view value) {
    return value.size() == 64 &&
           std::all_of(value.begin(), value.end(), [](const unsigned char byte) {
               return std::isdigit(byte) != 0 || (byte >= 'a' && byte <= 'f');
           });
}

bool decode_base64(const std::string_view input, std::vector<unsigned char>& output) {
    if (input.empty() || input.size() % 4 != 0 || input.size() > ((kMaximumCmsBytes + 2U) / 3U) * 4U)
        return false;
    std::array<int, 256> table{};
    table.fill(-1);
    constexpr std::string_view alphabet =
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for (std::size_t index = 0; index < alphabet.size(); ++index)
        table[static_cast<unsigned char>(alphabet[index])] = static_cast<int>(index);
    output.clear();
    output.reserve(input.size() / 4U * 3U);
    for (std::size_t offset = 0; offset < input.size(); offset += 4) {
        const bool final = offset + 4 == input.size();
        const char third = input[offset + 2];
        const char fourth = input[offset + 3];
        if ((!final && (third == '=' || fourth == '=')) || (third == '=' && fourth != '=')) return false;
        const int a = table[static_cast<unsigned char>(input[offset])];
        const int b = table[static_cast<unsigned char>(input[offset + 1])];
        const int c = third == '=' ? 0 : table[static_cast<unsigned char>(third)];
        const int d = fourth == '=' ? 0 : table[static_cast<unsigned char>(fourth)];
        if (a < 0 || b < 0 || c < 0 || d < 0) return false;
        if ((third == '=' && (b & 0x0f) != 0) || (fourth == '=' && third != '=' && (c & 0x03) != 0))
            return false;  // Reject non-canonical pad bits.
        const auto bits = static_cast<std::uint32_t>((a << 18) | (b << 12) | (c << 6) | d);
        output.push_back(static_cast<unsigned char>(bits >> 16U));
        if (third != '=') output.push_back(static_cast<unsigned char>(bits >> 8U));
        if (fourth != '=') output.push_back(static_cast<unsigned char>(bits));
    }
    return !output.empty() && output.size() <= kMaximumCmsBytes;
}

template <typename T>
bool get(const std::map<std::string, Value>& fields, const char* key, T& output) {
    const auto found = fields.find(key);
    if (found == fields.end()) return false;
    const auto* value = std::get_if<T>(&found->second);
    if (value == nullptr) return false;
    output = *value;
    return true;
}

#ifdef _WIN32
std::string utf8(const std::wstring_view text) {
    if (text.empty()) return {};
    const auto size = WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                                          static_cast<int>(text.size()), nullptr, 0, nullptr, nullptr);
    if (size <= 0) return {};
    std::string output(static_cast<std::size_t>(size), '\0');
    if (WideCharToMultiByte(CP_UTF8, WC_ERR_INVALID_CHARS, text.data(),
                            static_cast<int>(text.size()), output.data(), size,
                            nullptr, nullptr) != size) return {};
    return output;
}

std::string publisher_name(const PCCERT_CONTEXT certificate) {
    const auto size = CertGetNameStringW(certificate, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0,
                                         nullptr, nullptr, 0);
    if (size <= 1) return {};
    std::wstring value(static_cast<std::size_t>(size), L'\0');
    if (CertGetNameStringW(certificate, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, nullptr,
                           value.data(), size) != size) return {};
    value.resize(value.size() - 1U);
    return utf8(value);
}

std::string certificate_sha256(const PCCERT_CONTEXT certificate) {
    DWORD size = 0;
    if (!CertGetCertificateContextProperty(certificate, CERT_SHA256_HASH_PROP_ID, nullptr, &size) ||
        size == 0) return {};
    std::vector<unsigned char> digest(size);
    if (!CertGetCertificateContextProperty(certificate, CERT_SHA256_HASH_PROP_ID,
                                           digest.data(), &size)) return {};
    constexpr char hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(digest.size() * 2U);
    for (const auto byte : digest) {
        output.push_back(hex[byte >> 4U]);
        output.push_back(hex[byte & 0x0fU]);
    }
    return output;
}

bool code_signing_eku(const PCCERT_CONTEXT certificate) {
    DWORD size = 0;
    if (!CertGetEnhancedKeyUsage(certificate, 0, nullptr, &size) || size == 0) return false;
    std::vector<unsigned char> storage(size);
    auto* usage = reinterpret_cast<PCERT_ENHKEY_USAGE>(storage.data());
    if (!CertGetEnhancedKeyUsage(certificate, 0, usage, &size)) return false;
    for (DWORD index = 0; index < usage->cUsageIdentifier; ++index) {
        if (std::string_view(usage->rgpszUsageIdentifier[index]) == szOID_PKIX_KP_CODE_SIGNING)
            return true;
    }
    return false;
}

bool cms_uses_sha256(const std::vector<unsigned char>& cms) {
    HCRYPTMSG message = CryptMsgOpenToDecode(
        X509_ASN_ENCODING | PKCS_7_ASN_ENCODING, CMSG_DETACHED_FLAG, 0, 0, nullptr, nullptr);
    if (message == nullptr) return false;
    bool valid = CryptMsgUpdate(message, cms.data(), static_cast<DWORD>(cms.size()), TRUE) != FALSE;
    DWORD size = 0;
    if (!valid || !CryptMsgGetParam(message, CMSG_SIGNER_INFO_PARAM, 0, nullptr, &size) || size == 0)
        valid = false;
    std::vector<unsigned char> storage(size);
    if (valid && !CryptMsgGetParam(message, CMSG_SIGNER_INFO_PARAM, 0, storage.data(), &size))
        valid = false;
    if (valid) {
        const auto* signer = reinterpret_cast<const CMSG_SIGNER_INFO*>(storage.data());
        valid = signer->HashAlgorithm.pszObjId != nullptr &&
                std::string_view(signer->HashAlgorithm.pszObjId) == szOID_NIST_sha256;
    }
    CryptMsgClose(message);
    return valid;
}

std::string windows_error(const char* prefix) {
    return std::string(prefix) + " (Windows error " + std::to_string(GetLastError()) + ")";
}
#endif

}  // namespace

PackageSignatureResult parse_package_signature(const std::string_view json) {
    if (json.empty() || json.size() > kMaximumSignatureJsonBytes)
        return {false, {}, "signature metadata size is outside [1, 32768]"};
    if (json.size() >= 3 && static_cast<unsigned char>(json[0]) == 0xefU &&
        static_cast<unsigned char>(json[1]) == 0xbbU && static_cast<unsigned char>(json[2]) == 0xbfU)
        return {false, {}, "UTF-8 BOM is not accepted"};
    std::map<std::string, Value> fields;
    Parser parser(json);
    if (!parser.object(fields)) return {false, {}, parser.error()};
    if (fields.size() != 4) return {false, {}, "signature metadata must contain exactly four v1 fields"};
    std::uint64_t schema_version{};
    std::string base64;
    PackageSignature result;
    if (!get(fields, "schema_version", schema_version) ||
        !get(fields, "inventory_sha256", result.inventory_sha256) ||
        !get(fields, "format", result.format) || !get(fields, "signature_base64", base64))
        return {false, {}, "signature metadata is missing a required field"};
    if (schema_version != kPackageSignatureSchemaVersion)
        return {false, {}, "unsupported signature schema_version"};
    result.schema_version = static_cast<std::uint32_t>(schema_version);
    if (!sha256_text(result.inventory_sha256)) return {false, {}, "invalid inventory_sha256"};
    if (result.format != "cms-detached-sha256") return {false, {}, "unsupported signature format"};
    if (!decode_base64(base64, result.cms_der)) return {false, {}, "signature_base64 is not canonical bounded Base64"};
    if (result.cms_der.size() < 2 || result.cms_der.front() != 0x30U)
        return {false, {}, "CMS signature must be a DER SEQUENCE"};
    return {true, std::move(result), {}};
}

std::string package_signature_content(const std::string_view inventory_sha256) {
    if (!sha256_text(inventory_sha256)) return {};
    return "OwOPackageInventoryV1:" + std::string(inventory_sha256) + "\n";
}

SignedPackageMetadataResult inspect_signed_package_metadata(
    const std::filesystem::path& package_path) {
    return inspect_signed_package_metadata(inspect_package(package_path));
}

SignedPackageMetadataResult inspect_signed_package_metadata(const PackageInspection& package) {
    if (!package.ok) return {false, {}, {}, package.diagnostic};
    if (package.embedded_signature_json.empty())
        return {false, {}, {}, "root signature.json is required"};
    const auto signature = parse_package_signature(package.embedded_signature_json);
    if (!signature.ok) return {false, {}, {}, signature.diagnostic};
    if (signature.value.inventory_sha256 != package.inventory_sha256)
        return {false, {}, {}, "signature inventory_sha256 does not match package inventory"};
    return {true, package.inventory_sha256, signature.value, {}};
}

PackageTrustResult verify_package_signature_trust(const PackageSignature& signature) {
    PackageTrustResult result;
    result.inventory_sha256 = signature.inventory_sha256;
#ifdef _WIN32
    if (signature.format != "cms-detached-sha256" || !sha256_text(signature.inventory_sha256) ||
        signature.cms_der.empty() || signature.cms_der.size() > std::numeric_limits<DWORD>::max()) {
        result.diagnostic = "invalid package signature input";
        return result;
    }
    const auto content = package_signature_content(signature.inventory_sha256);
    if (content.empty() || content.size() > std::numeric_limits<DWORD>::max()) {
        result.diagnostic = "invalid detached signature content";
        return result;
    }
    const auto signer_count = CryptGetMessageSignerCount(
        X509_ASN_ENCODING | PKCS_7_ASN_ENCODING, signature.cms_der.data(),
        static_cast<DWORD>(signature.cms_der.size()));
    if (signer_count != 1) {
        result.diagnostic = signer_count < 0 ? windows_error("cannot inspect CMS signers")
                                             : "CMS must contain exactly one signer";
        return result;
    }
    if (!cms_uses_sha256(signature.cms_der)) {
        result.diagnostic = "CMS signer must use SHA-256";
        return result;
    }
    CRYPT_VERIFY_MESSAGE_PARA verify{};
    verify.cbSize = sizeof(verify);
    verify.dwMsgAndCertEncodingType = X509_ASN_ENCODING | PKCS_7_ASN_ENCODING;
    CERT_STRONG_SIGN_PARA strong_sign{};
    strong_sign.cbSize = sizeof(strong_sign);
    strong_sign.dwInfoChoice = CERT_STRONG_SIGN_OID_INFO_CHOICE;
    strong_sign.pszOID = const_cast<LPSTR>(szOID_CERT_STRONG_SIGN_OS_1);
    verify.pStrongSignPara = &strong_sign;
    const BYTE* content_parts[]{reinterpret_cast<const BYTE*>(content.data())};
    DWORD content_sizes[]{static_cast<DWORD>(content.size())};
    PCCERT_CONTEXT signer = nullptr;
    if (!CryptVerifyDetachedMessageSignature(
            &verify, 0, signature.cms_der.data(), static_cast<DWORD>(signature.cms_der.size()),
            1, content_parts, content_sizes, &signer) || signer == nullptr) {
        result.diagnostic = windows_error("detached CMS signature is invalid");
        if (signer != nullptr) CertFreeCertificateContext(signer);
        return result;
    }
    result.cryptographic_signature_valid = true;
    result.publisher_display_name = publisher_name(signer);
    result.certificate_sha256 = certificate_sha256(signer);
    if (!code_signing_eku(signer)) {
        result.diagnostic = "signer certificate does not explicitly allow code signing";
        CertFreeCertificateContext(signer);
        return result;
    }

    LPSTR usage_oid = const_cast<LPSTR>(szOID_PKIX_KP_CODE_SIGNING);
    CERT_CHAIN_PARA chain_parameters{};
    chain_parameters.cbSize = sizeof(chain_parameters);
    chain_parameters.RequestedUsage.dwType = USAGE_MATCH_TYPE_AND;
    chain_parameters.RequestedUsage.Usage.cUsageIdentifier = 1;
    chain_parameters.RequestedUsage.Usage.rgpszUsageIdentifier = &usage_oid;
    chain_parameters.pStrongSignPara = &strong_sign;
    PCCERT_CHAIN_CONTEXT chain = nullptr;
    constexpr DWORD chain_flags = CERT_CHAIN_REVOCATION_CHECK_CHAIN_EXCLUDE_ROOT |
                                  CERT_CHAIN_REVOCATION_CHECK_CACHE_ONLY |
                                  CERT_CHAIN_CACHE_ONLY_URL_RETRIEVAL |
                                  CERT_CHAIN_DISABLE_AIA;
    if (!CertGetCertificateChain(nullptr, signer, nullptr, signer->hCertStore, &chain_parameters,
                                 chain_flags, nullptr, &chain) || chain == nullptr) {
        result.diagnostic = windows_error("cannot build signer certificate chain");
        if (chain != nullptr) CertFreeCertificateChain(chain);
        CertFreeCertificateContext(signer);
        return result;
    }
    CERT_CHAIN_POLICY_PARA policy_parameters{};
    policy_parameters.cbSize = sizeof(policy_parameters);
    AUTHENTICODE_EXTRA_CERT_CHAIN_POLICY_PARA authenticode{};
    authenticode.cbSize = sizeof(authenticode);
    policy_parameters.pvExtraPolicyPara = &authenticode;
    CERT_CHAIN_POLICY_STATUS policy_status{};
    policy_status.cbSize = sizeof(policy_status);
    const bool policy_checked = CertVerifyCertificateChainPolicy(
        CERT_CHAIN_POLICY_AUTHENTICODE, chain, &policy_parameters, &policy_status) != FALSE;
    if (!policy_checked || policy_status.dwError != 0 || chain->TrustStatus.dwErrorStatus != CERT_TRUST_NO_ERROR) {
        result.diagnostic = !policy_checked ? windows_error("cannot evaluate Authenticode chain policy")
                                            : "signer certificate chain is not fully trusted with cached revocation status";
        CertFreeCertificateChain(chain);
        CertFreeCertificateContext(signer);
        return result;
    }
    CertFreeCertificateChain(chain);
    CertFreeCertificateContext(signer);
    result.ok = true;
    return result;
#else
    result.diagnostic = "Windows publisher trust verification is unavailable on this platform";
    return result;
#endif
}

PackageTrustResult verify_signed_package_trust(const std::filesystem::path& package_path) {
    return verify_signed_package_trust(inspect_package(package_path));
}

PackageTrustResult verify_signed_package_trust(const PackageInspection& package) {
    const auto metadata = inspect_signed_package_metadata(package);
    if (!metadata.ok) return {false, false, {}, {}, {}, metadata.diagnostic};
    return verify_package_signature_trust(metadata.signature);
}

}  // namespace owo::plugin
