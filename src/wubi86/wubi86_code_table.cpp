#include "owo/wubi86/wubi86_code_table.h"
#include <algorithm>
namespace owo::wubi86 {
namespace {
// Factual named-root/key relations checked against the source chart in SOURCES.md.
// Descriptive/non-Unicode root variants are deliberately not fabricated as Unicode.
constexpr RootGlyph glyphs[]{
#include "wubi86_roots.inc"
};
constexpr RootKey roots[]{
 {'g',11,"王","王戋五一"},{'f',12,"土","土士二干十寸雨"},{'d',13,"大","大犬三古石厂"},
 {'s',14,"木","木丁西"},{'a',15,"工","工戈七匚匸"},{'h',21,"目","目上止卜"},
 {'j',22,"日","日早虫刂"},{'k',23,"口","口川"},{'l',24,"田","田甲囗四车力"},
 {'m',25,"山","山由贝冂几"},{'t',31,"禾","禾竹丿彳夂攵"},{'r',32,"白","白手斤"},
 {'e',33,"月","月彡乃用"},{'w',34,"人","人八"},{'q',35,"金","金乂儿夕"},
 {'y',41,"言","言文方广丶"},{'u',42,"立","立辛六门疒"},{'i',43,"水","水小氵"},
 {'o',44,"火","火米灬"},{'p',45,"之","之宀冖辶廴"},{'n',51,"已","已巳己尸心羽"},
 {'b',52,"子","子耳了也凵"},{'v',53,"女","女刀九臼彐巛"},{'c',54,"又","又巴马厶"},
 {'x',55,"纟","弓匕幺纟"}};
}
Wubi86CodeTable::Wubi86CodeTable(const Wubi86Dictionary& dictionary) {
 for(const auto& entry:dictionary.entries()) {
  if(entry.kind==CodeKind::full)full_[entry.text].push_back(entry.code);
  if(entry.kind==CodeKind::short_code)short_[entry.text].push_back(entry.code);
 }
 for(auto* map:{&full_,&short_})for(auto& [text,codes]:*map) {std::sort(codes.begin(),codes.end());codes.erase(std::unique(codes.begin(),codes.end()),codes.end());}
}
std::vector<std::string> Wubi86CodeTable::full_codes(std::string_view character) const {auto it=full_.find(character);return it==full_.end()?std::vector<std::string>{}:it->second;}
std::vector<std::string> Wubi86CodeTable::short_codes(std::string_view character) const {auto it=short_.find(character);return it==short_.end()?std::vector<std::string>{}:it->second;}
std::span<const RootGlyph> Wubi86CodeTable::root_glyphs() { return glyphs; }
std::optional<RootGlyph> Wubi86CodeTable::root_glyph(std::string_view id) {
 for (const auto& glyph : glyphs) if (glyph.id == id) return glyph;
 return std::nullopt;
}
std::span<const RootKey> Wubi86CodeTable::keys() {return roots;}
std::optional<char> Wubi86CodeTable::root_key(std::string_view root) {
 const auto parts=split_utf8(root);if(!parts||parts->size()!=1)return std::nullopt;
 for(const auto& item:roots) {const auto named=split_utf8(item.roots);for(const auto& candidate:*named)if(candidate==root)return item.key;}
 return std::nullopt;
}
}
