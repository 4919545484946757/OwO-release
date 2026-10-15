#pragma once

#include "owo/plugin/package_archive.h"

#include <cstdint>
#include <filesystem>
#include <string>
#include <string_view>
#include <vector>

namespace owo::plugin {

inline constexpr std::uint32_t kPackageSignatureSchemaVersion = 1;

struct PackageSignature {
    std::uint32_t schema_version{};
    std::string inventory_sha256;
    std::string format;
    std::vector<unsigned char> cms_der;
};

struct PackageSignatureResult {
    bool ok{};
    PackageSignature value;
    std::string diagnostic;
};

/// Parses signature.json metadata only. Successful parsing does not establish publisher trust.
[[nodiscard]] PackageSignatureResult parse_package_signature(std::string_view json);

/// Domain-separated bytes supplied as detached CMS content.
[[nodiscard]] std::string package_signature_content(std::string_view inventory_sha256);

struct SignedPackageMetadataResult {
    bool ok{};
    std::string inventory_sha256;
    PackageSignature signature;
    std::string diagnostic;
};

/// Performs same-snapshot package preflight and signature metadata/digest matching.
/// This does not cryptographically verify CMS or establish publisher trust.
[[nodiscard]] SignedPackageMetadataResult inspect_signed_package_metadata(
    const std::filesystem::path& package_path);

/// Binds signature metadata to an already captured immutable package snapshot.
[[nodiscard]] SignedPackageMetadataResult inspect_signed_package_metadata(
    const PackageInspection& package);

struct PackageTrustResult {
    bool ok{};
    bool cryptographic_signature_valid{};
    std::string inventory_sha256;
    std::string publisher_display_name;
    std::string certificate_sha256;
    std::string diagnostic;
};

/// Verifies detached CMS and a strong Windows code-signing chain using only local stores,
/// embedded certificates, and cached revocation data. P3C does not accept timestamp fallback.
[[nodiscard]] PackageTrustResult verify_package_signature_trust(
    const PackageSignature& signature);

/// Same-snapshot metadata binding followed by CMS and Windows publisher trust verification.
[[nodiscard]] PackageTrustResult verify_signed_package_trust(
    const std::filesystem::path& package_path);

/// Verifies metadata binding and publisher trust without reopening the package path.
[[nodiscard]] PackageTrustResult verify_signed_package_trust(
    const PackageInspection& package);

}  // namespace owo::plugin
