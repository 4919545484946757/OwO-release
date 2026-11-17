#include "owo/engine/rime_lmdg.h"

#include <algorithm>
#include <array>

#ifdef _WIN32
#define NOMINMAX
#include <Windows.h>
#endif

namespace owo::engine {
namespace {

// Wire format references: lotem/librime-octagram (gram_db.h, gram_encoding.cc)
// and s-yata/darts-clone (DoubleArrayUnit). Only matched, in-bounds units are read.
constexpr std::size_t kOffsetField = 40;
constexpr std::size_t kHeaderSize = 44;
constexpr std::uint64_t kMaximumFileSize = 1024 * 1024 * 1024;
constexpr std::string_view kFormat = "Rime::Grammar/1.0";

std::uint32_t little32(const unsigned char* p) noexcept {
    return std::uint32_t{p[0]} | (std::uint32_t{p[1]} << 8) |
           (std::uint32_t{p[2]} << 16) | (std::uint32_t{p[3]} << 24);
}

std::uint32_t child_offset(const std::uint32_t unit) noexcept {
    return (unit >> 10) << ((unit & 0x200U) ? 8 : 0);
}

bool append_han(const std::string_view text, std::array<unsigned char, 18>& key,
                std::size_t& written) noexcept {
    for (std::size_t pos = 0; pos < text.size(); pos += 3) {
        const auto a = static_cast<unsigned char>(text[pos]);
        const auto b = static_cast<unsigned char>(text[pos + 1]);
        const auto c = static_cast<unsigned char>(text[pos + 2]);
        if (a < 0xe4 || a > 0xe9 || (b & 0xc0) != 0x80 || (c & 0xc0) != 0x80) return false;
        const auto scalar = ((a & 15) << 12) | ((b & 63) << 6) | (c & 63);
        if (scalar < 0x4000 || scalar > 0x9fff) return false;
        const auto high = static_cast<unsigned char>((scalar >> 8) + 0x40);
        const auto low = static_cast<unsigned char>(scalar & 255);
        if (low == 0) key[written++] = 0xe1;
        key[written++] = high;
        if (low != 0) key[written++] = low;
    }
    return true;
}

} // namespace

RimeLmdg::~RimeLmdg() { reset(); }

void RimeLmdg::reset() noexcept {
#ifdef _WIN32
    if (view_) UnmapViewOfFile(view_);
    if (file_) CloseHandle(file_);
#endif
    view_ = table_ = nullptr;
    file_ = nullptr;
    unit_count_ = 0;
}

std::string_view RimeLmdg::load(const std::filesystem::path& path) {
    reset();
#ifndef _WIN32
    (void)path;
    return "Rime grammar mapping not implemented on this platform";
#else
    if (!path.is_absolute() || path.root_name().native().size() != 2 ||
        path.root_name().native().back() != L':') return "model must have an absolute local drive path";
    const auto drive = GetDriveTypeW(path.root_path().c_str());
    if (drive != DRIVE_FIXED && drive != DRIVE_REMOVABLE) return "model must be on a local disk";
    // Keep the handle open without write sharing so another process cannot
    // truncate a model while the hot path is looking at its mapping.
    const auto file = CreateFileW(path.c_str(), GENERIC_READ, FILE_SHARE_READ, nullptr,
        OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL | FILE_FLAG_RANDOM_ACCESS, nullptr);
    if (file == INVALID_HANDLE_VALUE) return "cannot open model";
    file_ = file;
    const auto fail = [this](const std::string_view error) { reset(); return error; };
    LARGE_INTEGER size{};
    if (GetFileType(file) != FILE_TYPE_DISK || !GetFileSizeEx(file, &size) ||
        size.QuadPart < static_cast<LONGLONG>(kHeaderSize + 4) ||
        size.QuadPart > static_cast<LONGLONG>(kMaximumFileSize)) return fail("invalid model size");
    const auto mapped = CreateFileMappingW(file, nullptr, PAGE_READONLY, 0, 0, nullptr);
    if (!mapped) return fail("cannot create model mapping");
    view_ = static_cast<const unsigned char*>(MapViewOfFile(mapped, FILE_MAP_READ, 0, 0, 0));
    CloseHandle(mapped);
    if (!view_) return fail("cannot map model");
    if (std::string_view(reinterpret_cast<const char*>(view_), kFormat.size()) != kFormat ||
        view_[kFormat.size()] != 0) return fail("unsupported Rime grammar format");
    const auto relative = little32(view_ + kOffsetField);
    const auto declared_count = little32(view_ + 36);
    if (relative < 4 || relative % 4 || !declared_count) return fail("invalid model table header");
    const auto table_pos = std::uint64_t{kOffsetField} + relative;
    if (table_pos + std::uint64_t{declared_count} * 4 > static_cast<std::uint64_t>(size.QuadPart))
        return fail("model table is truncated");
    table_ = view_ + static_cast<std::size_t>(table_pos);
    unit_count_ = declared_count;
    const auto root = little32(table_);
    if ((root & 0x800001ffU) || child_offset(root) >= unit_count_)
        return fail("invalid model root unit");
    return {};
#endif
}

std::int64_t RimeLmdg::boundary_bonus(const std::string_view left,
                                     const std::string_view right) const noexcept {
    if (!ready() || left.size() < 6 || right.size() < 6 || left.size() > 12 || right.size() > 12 ||
        left.size() % 3 || right.size() % 3 || left.size() + right.size() > 18) return 0;
    std::array<unsigned char, 18> key{};
    std::size_t length = 0;
    if (!append_han(left, key, length) || !append_han(right, key, length)) return 0;
    std::uint32_t position = 0;
    auto unit = little32(table_);
    for (std::size_t i = 0; i < length; ++i) {
        position ^= child_offset(unit) ^ key[i];
        if (position >= unit_count_) return 0;
        unit = little32(table_ + std::size_t{position} * 4);
        if ((unit & 0x800000ffU) != key[i]) return 0;
    }
    if (!(unit & 0x100U)) return 0;
    position ^= child_offset(unit);
    if (position >= unit_count_) return 0;
    const auto leaf = little32(table_ + std::size_t{position} * 4);
    if (!(leaf & 0x80000000U)) return 0;
    const auto value = static_cast<std::int64_t>(leaf & 0x7fffffffU);
    // Fixed positive evidence only. Raw weights are 10,000*log(count);
    // do not interpret missing phrases as ungrammatical or penalize them.
    return std::clamp<std::int64_t>((value - 135'000) / 8, 0, kMaximumBonus);
}

} // namespace owo::engine
