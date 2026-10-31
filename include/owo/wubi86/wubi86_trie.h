#pragma once
#include "wubi86_types.h"
#include <array>
#include <limits>
namespace owo::wubi86 {
class Wubi86Trie {
public:
 Wubi86Trie();
 // Duplicate identity is (code,text); retain highest frequency. False = duplicate or invalid.
 bool insert(const Entry& entry);
 [[nodiscard]] std::vector<Entry> exact(std::string_view code) const;
 [[nodiscard]] std::vector<Entry> prefix(std::string_view code, std::size_t limit = (std::numeric_limits<std::size_t>::max)()) const;
private:
 friend class Wubi86Dictionary;
 bool insert_validated(const Entry& entry);
 struct Node { std::array<std::size_t,25> children{}; std::vector<Entry> entries; };
 std::vector<Node> nodes_;
 [[nodiscard]] std::size_t find(std::string_view code) const;
};
}
