#include "owo/wubi86/wubi86_code_table.h"
#include "owo/wubi86/wubi86_encoder.h"
#include "owo/wubi86/wubi86_dictionary.h"
#include <algorithm>
#include <iostream>
#include <sstream>
#include <stdexcept>
using namespace owo::wubi86;
#define CHECK(x) do { if (!(x)) throw std::runtime_error("line " + std::to_string(__LINE__) + ": " #x); } while(false)
int main(int argc, char** argv) {
 try {
  CHECK(argc == 2);
  Wubi86Dictionary dictionary;
  auto loaded = dictionary.load_file(argv[1]);
  CHECK(loaded.ok); CHECK(dictionary.size() > 80000);
  Wubi86CodeTable table(dictionary);
  Wubi86Encoder encoder(table);
  // Golden records from pinned KyleBing main table v4.3; see data/wubi86/SOURCES.md.
  CHECK(table.full_codes("你") == std::vector<std::string>{"wqiy"});
  CHECK(table.short_codes("你") == (std::vector<std::string>{"wq", "wqi"}));
  CHECK(table.full_codes("一") == std::vector<std::string>{"ggll"});
  CHECK(table.full_codes("戈") == (std::vector<std::string>{"agnt", "agny"}));
  CHECK(table.full_codes("乙") == std::vector<std::string>{"nnll"});
  CHECK(encoder.encode("工期").codes == std::vector<std::string>{"aaad"});
  CHECK(encoder.encode("黄花菜").codes == std::vector<std::string>{"aaae"});
  CHECK(encoder.encode("恭恭敬敬").codes == std::vector<std::string>{"aaaa"});
  CHECK(encoder.encode("恭恭敬敬工").codes == std::vector<std::string>{"aaaa"});
  CHECK(table.full_codes("艹") == std::vector<std::string>{"aghh"});
  CHECK(encoder.encode("艹工").codes == std::vector<std::string>{"agaa"}); // Never the copp component alias.
  CHECK(table.full_codes("中") == std::vector<std::string>{"khk"});
  CHECK(encoder.encode("中国").codes == std::vector<std::string>{"khlg"});
  CHECK(!encoder.encode("").ok); CHECK(!encoder.encode("🙂").ok);
  CHECK(!encoder.encode(std::string("\xc0\xaf",2)).ok);
  CHECK(!encoder.encode(std::string("\xed\xa0\x80",3)).ok);
  CHECK(!encoder.encode(std::string("\xf4\x90\x80\x80",4)).ok);
  CHECK(valid_code("abcd"));
  for (auto s : {"", "z", "AB", "abcde", "a1", "a b"}) CHECK(!valid_code(s));
  CHECK(Wubi86CodeTable::keys().size() == 25);
  for (const auto& key : Wubi86CodeTable::keys()) {
   CHECK(key.position >= 11 && key.position <= 55);
   CHECK(Wubi86CodeTable::root_key(key.key_name) == key.key);
  }
  CHECK(Wubi86CodeTable::root_key("戋") == 'g');
  CHECK(Wubi86CodeTable::root_key("冂") == 'm');
  CHECK(Wubi86CodeTable::root_key("彐") == 'v');
  CHECK(!Wubi86CodeTable::root_key("🙂"));
  // Synthetic fixtures below test mechanics ONLY, not Wubi standard correctness.
  Wubi86Trie trie;
  CHECK(trie.insert({"ab", "甲", 10, CodeKind::unclassified}));
  CHECK(trie.insert({"ab", "乙", 20, CodeKind::unclassified}));
  CHECK(!trie.insert({"ab", "甲", 30, CodeKind::unclassified}));
  CHECK(trie.insert({"abc", "丙", 30, CodeKind::unclassified}));
  CHECK(trie.exact("ab").size() == 2);
  CHECK(trie.exact("ab")[0].text == "甲");
  CHECK(trie.prefix("a").size() == 3);
  CHECK(trie.prefix("a")[0].code == "ab"); // frequency tie: code, then UTF-8 text.
  CHECK(trie.prefix("a", 1).size() == 1);
  for(auto s : {"", "z", "A", "aaaaa", "yy"}) CHECK(trie.exact(s).empty());
  CHECK(trie.prefix("").empty()); CHECK(trie.prefix("x").empty());
  CHECK(!trie.insert({"z", "甲", 1, CodeKind::full}));
  const std::string header = "OWO-WUBI86\t1\nversion\tsynthetic-v1\nsource\tsynthetic-tests\n...\n";
  Wubi86Dictionary fixture;
  std::istringstream good(header + "abcd\t甲\t10\tfull\nab\t甲\t20\tshort\nabce\t甲\t9\tfull\nabcd\t甲\t30\tfull\n");
  auto result = fixture.load(good); CHECK(result.ok); CHECK(result.duplicates == 1); CHECK(fixture.size()==3);
  Wubi86CodeTable multi(fixture); Wubi86Encoder multi_encoder(multi);
  CHECK(multi.full_codes("甲").size()==2);
  CHECK(multi.short_codes("甲").size()==1);
  CHECK(!multi_encoder.encode("甲", 1).ok);
  CHECK(!multi_encoder.encode("甲", 0).ok);
  CHECK(split_utf8("𠀀")->size()==1);
  CHECK(!split_utf8(std::string("a\0b",3)));
  CHECK(!split_utf8(std::string("\xe4\xb8",2)));
  std::istringstream branching(header + "abcd\t甲\t1\tfull\ncdab\t甲\t1\tfull\n");
  Wubi86Dictionary branches; CHECK(branches.load(branching).ok);
  Wubi86Encoder branch_encoder{Wubi86CodeTable(branches)};
  CHECK(branch_encoder.encode("甲甲").codes == (std::vector<std::string>{"abab","abcd","cdab","cdcd"}));
  CHECK(!branch_encoder.encode("甲甲",3).ok);
  CHECK(multi_encoder.encode("甲甲").codes == std::vector<std::string>{"abab"});
  CHECK(fixture.trie().exact("abcd")[0].frequency==30);
  for(const auto& bad : {"z\t甲\t1\tfull\n", "ab\t甲\t-1\tfull\n", "ab\t甲\t4294967296\tfull\n", "ab\t甲\t1\tbogus\n", "ab\t甲\t1\tfull\textra\n", "ab\t\t1\tfull\n", "ab\t甲\t1x\tfull\n", "ab\t甲乙\t1\tfull\n", "abcd\t甲\t1\tshort\n"}) {
   std::istringstream broken(header + bad); CHECK(!fixture.load(broken).ok); CHECK(fixture.size()==3);
  }
  std::istringstream corrupt(header + "ab\t" + std::string("\x80",1) + "\t1\tfull\n");
  CHECK(!fixture.load(corrupt).ok);
  std::istringstream version("OWO-WUBI86\t2\n"); CHECK(!fixture.load(version).ok);
  CHECK(!fixture.load_file("nonexistent-wubi-file").ok);
  std::istringstream conflicting(header + "ab\t甲\t1\tfull\nab\t甲\t2\tshort\n");
  CHECK(!fixture.load(conflicting).ok); CHECK(fixture.size()==3);
  std::istringstream empty_dictionary(header); CHECK(!fixture.load(empty_dictionary).ok);
  std::istringstream broken_stream(header); broken_stream.setstate(std::ios::badbit);
  CHECK(!fixture.load(broken_stream).ok);
  // Scale test uses actual normalized source, not generated pseudo-standard data.
  std::size_t count=0;
  for(const auto& entry : dictionary.entries()) {
   const auto exact = dictionary.trie().exact(entry.code);
   CHECK(std::any_of(exact.begin(),exact.end(),[&](const auto& e){return e.text==entry.text;}));
   if(++count==20000) break;
  }
  CHECK(count==20000);
  std::cout << "PASS: golden, roots, phrase rules, UTF-8, validation, multi-code, trie, loading, 20000 lookups; entries=" << dictionary.size() << '\n';
  return 0;
 } catch(const std::exception& e) { std::cerr << e.what() << '\n'; return 1; }
}
