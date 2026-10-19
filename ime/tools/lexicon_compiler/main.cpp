#include "owo/engine/binary_lexicon.h"

#ifdef _WIN32
#include <Windows.h>
#include <bcrypt.h>
#endif

#include <algorithm>
#include <charconv>
#include <fstream>
#include <iostream>
#include <map>
#include <sstream>
#include <string>
#include <unordered_map>
#include <unordered_set>

namespace {

std::string trim_cr(std::string value) {
    if (!value.empty() && value.back() == '\r') value.pop_back();
    return value;
}

bool valid_sha256(const std::string& value) {
    if (value.size() != 64) return false;
    return value.find_first_not_of("0123456789abcdefABCDEF") == std::string::npos;
}

bool read_manifest(const char* path, std::string& source_sha256, std::string& source_format,
                   std::string& pronunciation_sha256) {
    std::ifstream input(path);
    if (!input) return false;
    std::map<std::string, std::string> fields;
    std::string line;
    while (std::getline(input, line)) {
        line = trim_cr(std::move(line));
        if (line.empty() || line.front() == '#') continue;
        const auto separator = line.find('=');
        if (separator == std::string::npos || separator == 0 || separator + 1 == line.size()) return false;
        if (!fields.emplace(line.substr(0, separator), line.substr(separator + 1)).second) return false;
    }
    static const std::unordered_set<std::string> allowed_fields{
        "source_id", "source_version", "source_sha256", "license",
        "source_format", "source_url", "importer", "pronunciation_source",
        "pronunciation_sha256"};
    const bool known_fields = std::all_of(fields.begin(), fields.end(), [](const auto& field) {
        return allowed_fields.contains(field.first);
    });
    const bool valid = fields.size() >= 4 && fields.size() <= allowed_fields.size() && known_fields &&
           fields.contains("source_id") &&
           fields.contains("source_version") && fields.contains("source_sha256") &&
           fields.contains("license") && valid_sha256(fields["source_sha256"]) &&
           (!fields.contains("source_format") || fields["source_format"] == "rime-dict-yaml" ||
            fields["source_format"] == "rime-dict-yaml-auto" ||
            fields["source_format"] == "rime-dict-yaml-mixed") &&
           (!fields.contains("pronunciation_sha256") ||
            valid_sha256(fields["pronunciation_sha256"]));
    if (valid) {
        source_sha256 = fields["source_sha256"];
        source_format = fields.contains("source_format") ? fields["source_format"] : "tsv";
        pronunciation_sha256 = fields.contains("pronunciation_sha256")
            ? fields["pronunciation_sha256"] : "";
    }
    return valid;
}

std::string sha256_file(const char* path) {
#ifdef _WIN32
    std::ifstream input(path, std::ios::binary);
    if (!input) return {};
    std::vector<unsigned char> bytes((std::istreambuf_iterator<char>(input)), {});
    BCRYPT_ALG_HANDLE algorithm = nullptr;
    BCRYPT_HASH_HANDLE hash = nullptr;
    DWORD hash_size = 0, result_size = 0;
    if (BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) < 0 ||
        BCryptGetProperty(algorithm, BCRYPT_HASH_LENGTH,
                          reinterpret_cast<PUCHAR>(&hash_size), sizeof(hash_size),
                          &result_size, 0) < 0 ||
        BCryptCreateHash(algorithm, &hash, nullptr, 0, nullptr, 0, 0) < 0 ||
        BCryptHashData(hash, bytes.data(), static_cast<ULONG>(bytes.size()), 0) < 0) {
        if (hash != nullptr) BCryptDestroyHash(hash);
        if (algorithm != nullptr) BCryptCloseAlgorithmProvider(algorithm, 0);
        return {};
    }
    std::vector<unsigned char> digest(hash_size);
    const bool finished = BCryptFinishHash(hash, digest.data(), hash_size, 0) >= 0;
    BCryptDestroyHash(hash);
    BCryptCloseAlgorithmProvider(algorithm, 0);
    if (!finished) return {};
    constexpr char hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(digest.size() * 2);
    for (const auto byte : digest) {
        output.push_back(hex[byte >> 4U]);
        output.push_back(hex[byte & 0x0fU]);
    }
    return output;
#else
    static_cast<void>(path);
    return {};
#endif
}

using Pronunciations = std::unordered_map<std::string, std::vector<std::string>>;

bool split_utf8(const std::string& text, std::vector<std::string>& characters) {
    for (std::size_t offset = 0; offset < text.size();) {
        const auto lead = static_cast<unsigned char>(text[offset]);
        std::size_t length = lead < 0x80 ? 1 : (lead & 0xe0) == 0xc0 ? 2 :
                             (lead & 0xf0) == 0xe0 ? 3 : (lead & 0xf8) == 0xf0 ? 4 : 0;
        if (length == 0 || offset + length > text.size()) return false;
        for (std::size_t index = 1; index < length; ++index)
            if ((static_cast<unsigned char>(text[offset + index]) & 0xc0) != 0x80) return false;
        characters.emplace_back(text.substr(offset, length));
        offset += length;
    }
    return !characters.empty();
}

bool parse_entries(const char* path, const std::string& source_format,
                   const Pronunciations* pronunciations,
                   std::vector<owo::engine::LexiconEntry>& entries) {
    std::ifstream input(path, std::ios::binary);
    if (!input) return false;
    std::unordered_set<std::string> unique;
    std::string line;
    std::size_t line_number = 0;
    bool rime_body = source_format != "rime-dict-yaml" &&
                     source_format != "rime-dict-yaml-auto" &&
                     source_format != "rime-dict-yaml-mixed";
    while (std::getline(input, line)) {
        ++line_number;
        line = trim_cr(std::move(line));
        if (!rime_body) {
            if (line == "...") rime_body = true;
            continue;
        }
        if (line.empty() || line.front() == '#') continue;
        const auto first = line.find('\t');
        const auto second = first == std::string::npos ? first : line.find('\t', first + 1);
        const bool auto_annotate = source_format == "rime-dict-yaml-auto";
        const bool mixed = source_format == "rime-dict-yaml-mixed";
        const auto tail = first == std::string::npos ? std::string_view{} :
                          std::string_view(line).substr(first + 1);
        const bool numeric_tail = !tail.empty() &&
            tail.find_first_not_of("0123456789") == std::string_view::npos;
        const bool auto_row = auto_annotate || (mixed && (first == std::string::npos || numeric_tail));
        const bool rime_without_frequency = !auto_row &&
            (source_format == "rime-dict-yaml" || mixed) &&
            first != std::string::npos && second == std::string::npos;
        if ((!auto_row && first == std::string::npos) ||
            (!rime_without_frequency && !auto_row && second == std::string::npos) ||
            (second != std::string::npos && line.find('\t', second + 1) != std::string::npos)) {
            std::cerr << "invalid lexicon row at line " << line_number << '\n';
            return false;
        }
        owo::engine::LexiconEntry entry;
        entry.text = first == std::string::npos ? line : line.substr(0, first);
        if (auto_row) {
            if (second != std::string::npos || pronunciations == nullptr) return false;
            std::vector<std::string> characters;
            if (!split_utf8(entry.text, characters)) return false;
            std::vector<std::vector<std::string>> combinations(1);
            for (const auto& character : characters) {
                const auto found = pronunciations->find(character);
                if (found == pronunciations->end()) {
                    std::cerr << "unresolved pronunciation at line " << line_number << '\n';
                    return false;
                }
                std::vector<std::vector<std::string>> expanded;
                if (combinations.size() * found->second.size() > 64) {
                    std::cerr << "pronunciation expansion exceeds limit at line " << line_number << '\n';
                    return false;
                }
                for (const auto& combination : combinations) {
                    for (const auto& reading : found->second) {
                        auto next = combination;
                        next.push_back(reading);
                        expanded.push_back(std::move(next));
                    }
                }
                combinations = std::move(expanded);
            }
            entry.frequency = 1;
            if (first != std::string::npos) {
                const auto frequency_text = std::string_view(line).substr(first + 1);
                const auto result = std::from_chars(frequency_text.data(),
                                                    frequency_text.data() + frequency_text.size(),
                                                    entry.frequency);
                if (result.ec != std::errc{} ||
                    result.ptr != frequency_text.data() + frequency_text.size()) return false;
            }
            for (auto& combination : combinations) {
                auto expanded_entry = entry;
                expanded_entry.syllables = std::move(combination);
                std::string key;
                for (const auto& syllable : expanded_entry.syllables) key += syllable + '\0';
                key += expanded_entry.text;
                if (unique.insert(std::move(key)).second)
                    entries.push_back(std::move(expanded_entry));
            }
            continue;
        } else {
        const auto reading_end = rime_without_frequency ? line.size() : second;
        std::istringstream reading(line.substr(first + 1, reading_end - first - 1));
        for (std::string syllable; reading >> syllable;) {
            if (syllable.find_first_not_of("abcdefghijklmnopqrstuvwxyz") != std::string::npos) return false;
            entry.syllables.push_back(std::move(syllable));
        }
        entry.frequency = 1;
        if (!rime_without_frequency) {
            const auto frequency_text = std::string_view(line).substr(second + 1);
            const auto result = std::from_chars(frequency_text.data(),
                                                frequency_text.data() + frequency_text.size(),
                                                entry.frequency);
            if (result.ec != std::errc{} ||
                result.ptr != frequency_text.data() + frequency_text.size()) return false;
        }
        }
        if (entry.text.empty() || entry.syllables.empty()) return false;
        std::string key;
        for (const auto& syllable : entry.syllables) key += syllable + '\0';
        key += entry.text;
        if (!unique.insert(std::move(key)).second) {
            std::cerr << "duplicate entry at line " << line_number << '\n';
            return false;
        }
        entries.push_back(std::move(entry));
    }
    return rime_body && !entries.empty();
}

bool load_pronunciations(const char* path, Pronunciations& pronunciations) {
    std::vector<owo::engine::LexiconEntry> entries;
    if (!parse_entries(path, "rime-dict-yaml", nullptr, entries)) return false;
    struct Choice { std::uint32_t maximum{}; std::vector<std::pair<std::string, std::uint32_t>> readings; };
    std::unordered_map<std::string, Choice> choices;
    for (const auto& entry : entries) {
        std::vector<std::string> characters;
        if (entry.syllables.size() != 1 || !split_utf8(entry.text, characters) || characters.size() != 1)
            continue;
        auto& choice = choices[entry.text];
        choice.maximum = (std::max)(choice.maximum, entry.frequency);
        choice.readings.emplace_back(entry.syllables.front(), entry.frequency);
    }
    for (auto& [character, choice] : choices) {
        std::vector<std::string> selected;
        for (auto& [reading, frequency] : choice.readings)
            if (static_cast<std::uint64_t>(frequency) * 100U >=
                static_cast<std::uint64_t>(choice.maximum) * 5U)
                selected.push_back(std::move(reading));
        if (!selected.empty()) pronunciations.emplace(std::move(character), std::move(selected));
    }
    return !pronunciations.empty();
}

}  // namespace

int main(int argc, char** argv) {
    if (argc != 4 && argc != 5) {
        std::cerr << "usage: owo_lexicon_compiler <source> <source.manifest> <output.owolx> [pronunciations]\n";
        return 2;
    }
    std::string expected_sha256;
    std::string source_format;
    std::string pronunciation_sha256;
    if (!read_manifest(argv[2], expected_sha256, source_format, pronunciation_sha256)) {
        std::cerr << "invalid source manifest\n";
        return 3;
    }
    if (sha256_file(argv[1]) != expected_sha256) {
        std::cerr << "source SHA-256 does not match manifest\n";
        return 3;
    }
    const bool auto_annotate = source_format == "rime-dict-yaml-auto" ||
                               source_format == "rime-dict-yaml-mixed";
    if (auto_annotate != (argc == 5) ||
        (auto_annotate && (pronunciation_sha256.empty() ||
                           sha256_file(argv[4]) != pronunciation_sha256))) {
        std::cerr << "invalid pronunciation source\n";
        return 3;
    }
    Pronunciations pronunciations;
    if (auto_annotate && !load_pronunciations(argv[4], pronunciations)) return 4;
    std::vector<owo::engine::LexiconEntry> entries;
    if (!parse_entries(argv[1], source_format, auto_annotate ? &pronunciations : nullptr, entries)) return 4;
    const auto result = owo::engine::write_binary_lexicon(argv[3], std::move(entries));
    if (!result.success) {
        std::cerr << result.error << '\n';
        return 5;
    }
    return 0;
}
