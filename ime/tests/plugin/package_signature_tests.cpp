#include "owo/plugin/package_signature.h"

#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#include <wincrypt.h>

#include <string>
#include <iostream>
#include <vector>

namespace {

const std::string digest(64, 'a');

std::string valid() {
    return "{\"schema_version\":1,\"inventory_sha256\":\"" + digest +
           "\",\"format\":\"cms-detached-sha256\",\"signature_base64\":\"MAMCAQE=\"}";
}

bool rejected(std::string json, const std::string& from, const std::string& to) {
    const auto position = json.find(from);
    if (position == std::string::npos) return false;
    json.replace(position, from.size(), to);
    return !owo::plugin::parse_package_signature(json).ok;
}

std::vector<unsigned char> create_untrusted_cms(const std::string& content,
                                                const char* hash_oid = szOID_NIST_sha256,
                                                const DWORD key_bits = 2048U) {
    HCRYPTPROV provider = 0;
    HCRYPTKEY key = 0;
    PCCERT_CONTEXT certificate = nullptr;
    BYTE* encoded_usage = nullptr;
    const auto container = L"OwO.Plugin.Signature.Test." + std::to_wstring(GetCurrentProcessId()) +
                           L"." + std::to_wstring(GetTickCount64());
    const char* stage = "acquire provider";
    std::vector<unsigned char> cms;
    do {
        if (!CryptAcquireContextW(&provider, container.c_str(), MS_ENH_RSA_AES_PROV_W, PROV_RSA_AES,
                                  CRYPT_NEWKEYSET | CRYPT_SILENT)) break;
        stage = "generate key";
        if (!CryptGenKey(provider, AT_SIGNATURE, (key_bits << 16U) | CRYPT_EXPORTABLE, &key)) break;
        stage = "encode subject size";
        DWORD subject_size = 0;
        constexpr wchar_t subject[] = L"CN=OwO Untrusted Test Publisher";
        if (!CertStrToNameW(X509_ASN_ENCODING, subject, CERT_X500_NAME_STR, nullptr,
                            nullptr, &subject_size, nullptr)) break;
        stage = "encode subject";
        std::vector<unsigned char> subject_bytes(subject_size);
        if (!CertStrToNameW(X509_ASN_ENCODING, subject, CERT_X500_NAME_STR, nullptr,
                            subject_bytes.data(), &subject_size, nullptr)) break;
        CERT_NAME_BLOB subject_blob{subject_size, subject_bytes.data()};

        stage = "encode EKU";
        LPSTR code_signing_oid = const_cast<LPSTR>(szOID_PKIX_KP_CODE_SIGNING);
        CERT_ENHKEY_USAGE usage{1, &code_signing_oid};
        DWORD encoded_usage_size = 0;
        if (!CryptEncodeObjectEx(X509_ASN_ENCODING, X509_ENHANCED_KEY_USAGE, &usage,
                                 CRYPT_ENCODE_ALLOC_FLAG, nullptr, &encoded_usage,
                                 &encoded_usage_size)) break;
        CERT_EXTENSION extension{};
        extension.pszObjId = const_cast<LPSTR>(szOID_ENHANCED_KEY_USAGE);
        extension.Value = {encoded_usage_size, encoded_usage};
        CERT_EXTENSIONS extensions{1, &extension};
        CRYPT_ALGORITHM_IDENTIFIER certificate_algorithm{};
        certificate_algorithm.pszObjId = const_cast<LPSTR>(szOID_RSA_SHA256RSA);
        CRYPT_KEY_PROV_INFO key_provider_info{};
        key_provider_info.pwszContainerName = const_cast<LPWSTR>(container.c_str());
        key_provider_info.pwszProvName = const_cast<LPWSTR>(MS_ENH_RSA_AES_PROV_W);
        key_provider_info.dwProvType = PROV_RSA_AES;
        key_provider_info.dwKeySpec = AT_SIGNATURE;
        stage = "create certificate";
        certificate = CertCreateSelfSignCertificate(
            provider, &subject_blob, 0, &key_provider_info,
            &certificate_algorithm, nullptr, nullptr, &extensions);
        if (certificate == nullptr) break;

        stage = "size CMS";
        CRYPT_SIGN_MESSAGE_PARA parameters{};
        parameters.cbSize = sizeof(parameters);
        parameters.dwMsgEncodingType = X509_ASN_ENCODING | PKCS_7_ASN_ENCODING;
        parameters.pSigningCert = certificate;
        parameters.HashAlgorithm.pszObjId = const_cast<LPSTR>(hash_oid);
        PCCERT_CONTEXT certificates[]{certificate};
        parameters.cMsgCert = 1;
        parameters.rgpMsgCert = certificates;
        parameters.dwFlags = CRYPT_MESSAGE_SILENT_KEYSET_FLAG;
        const BYTE* parts[]{reinterpret_cast<const BYTE*>(content.data())};
        DWORD sizes[]{static_cast<DWORD>(content.size())};
        DWORD cms_size = 0;
        if (!CryptSignMessage(&parameters, TRUE, 1, parts, sizes, nullptr, &cms_size) ||
            cms_size == 0) break;
        stage = "sign CMS";
        cms.resize(cms_size);
        if (!CryptSignMessage(&parameters, TRUE, 1, parts, sizes, cms.data(), &cms_size)) {
            cms.clear();
            break;
        }
        cms.resize(cms_size);
    } while (false);
    DWORD failure_error = cms.empty() ? GetLastError() : ERROR_SUCCESS;
    if (encoded_usage != nullptr) LocalFree(encoded_usage);
    if (key != 0) CryptDestroyKey(key);
    if (certificate != nullptr) CertFreeCertificateContext(certificate);
    if (provider != 0) CryptReleaseContext(provider, 0);
    HCRYPTPROV deleted = 0;
    if (!CryptAcquireContextW(&deleted, container.c_str(), MS_ENH_RSA_AES_PROV_W, PROV_RSA_AES,
                              CRYPT_DELETEKEYSET | CRYPT_SILENT)) {
        const auto delete_error = GetLastError();
        // Some provider versions remove this transient container when the final handle closes.
        // NTE_BAD_KEYSET therefore already represents the required clean postcondition.
        if (delete_error != NTE_BAD_KEYSET) {
            stage = "delete key container";
            failure_error = delete_error;
            cms.clear();
        }
    }
    if (cms.empty()) std::cerr << "CMS fixture failed at " << stage << ", error=" << failure_error << '\n';
    return cms;
}

}  // namespace

int main() {
    const auto parsed = owo::plugin::parse_package_signature(valid());
    if (!parsed.ok || parsed.value.schema_version != 1 || parsed.value.cms_der.size() != 5 ||
        parsed.value.inventory_sha256 != digest) return 1;
    if (owo::plugin::package_signature_content(digest) !=
        "OwOPackageInventoryV1:" + digest + "\n") return 2;
    if (!owo::plugin::package_signature_content("bad").empty()) return 3;
    if (!rejected(valid(), "\"schema_version\":1", "\"schema_version\":2")) return 4;
    if (!rejected(valid(), "cms-detached-sha256", "rsa-sha256")) return 5;
    if (!rejected(valid(), std::string(64, 'a'), std::string(63, 'a') + "g")) return 6;
    if (!rejected(valid(), "MAMCAQE=", "MAMCAQF=")) return 7;  // non-canonical pad bits
    if (!rejected(valid(), "MAMCAQE=", "AQIDBA==")) return 8;  // not a DER sequence
    if (!rejected(valid(), "\"format\"", "\"unknown\":false,\"format\"")) return 9;
    if (!rejected(valid(), "\"format\":", "\"format\":\"cms-detached-sha256\",\"format\":")) return 10;
    if (owo::plugin::parse_package_signature(valid() + "x").ok) return 11;
    const auto trust = owo::plugin::verify_package_signature_trust(parsed.value);
    if (trust.ok || trust.cryptographic_signature_valid) return 12;
    owo::plugin::PackageSignature untrusted;
    untrusted.schema_version = 1;
    untrusted.inventory_sha256 = digest;
    untrusted.format = "cms-detached-sha256";
    untrusted.cms_der = create_untrusted_cms(owo::plugin::package_signature_content(digest));
    if (untrusted.cms_der.empty()) return 13;
    const auto untrusted_result = owo::plugin::verify_package_signature_trust(untrusted);
    if (untrusted_result.ok || !untrusted_result.cryptographic_signature_valid ||
        untrusted_result.publisher_display_name != "OwO Untrusted Test Publisher" ||
        untrusted_result.certificate_sha256.size() != 64) return 14;
    untrusted.inventory_sha256 = std::string(64, 'b');
    const auto tampered = owo::plugin::verify_package_signature_trust(untrusted);
    if (tampered.ok || tampered.cryptographic_signature_valid) return 15;
    untrusted.inventory_sha256 = digest;
    untrusted.cms_der = create_untrusted_cms(owo::plugin::package_signature_content(digest),
                                             szOID_OIWSEC_sha1);
    if (untrusted.cms_der.empty()) return 16;
    const auto sha1 = owo::plugin::verify_package_signature_trust(untrusted);
    if (sha1.ok || sha1.cryptographic_signature_valid) return 17;
    untrusted.cms_der = create_untrusted_cms(
        owo::plugin::package_signature_content(digest), szOID_NIST_sha256, 1024U);
    if (untrusted.cms_der.empty()) return 18;
    const auto weak_key = owo::plugin::verify_package_signature_trust(untrusted);
    if (weak_key.ok || weak_key.cryptographic_signature_valid) return 19;
    return 0;
}
