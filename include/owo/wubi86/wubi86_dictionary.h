#pragma once
#include "wubi86_trie.h"
#include <filesystem>
#include <istream>
namespace owo::wubi86 {
class Wubi86Dictionary {
public:
 // Transactional: malformed files do not replace the previously loaded dictionary.
 LoadResult load(std::istream& input);
 LoadResult load_file(const std::filesystem::path& path);
 [[nodiscard]] const Wubi86Trie& trie() const noexcept { return trie_; }
 [[nodiscard]] const std::vector<Entry>& entries() const noexcept { return entries_; }
 [[nodiscard]] std::size_t size() const noexcept { return entries_.size(); }
 [[nodiscard]] const std::string& version() const noexcept { return version_; }
 [[nodiscard]] const std::string& source() const noexcept { return source_; }
private:
 Wubi86Trie trie_;
 std::vector<Entry> entries_;
 std::string version_, source_;
};
}
