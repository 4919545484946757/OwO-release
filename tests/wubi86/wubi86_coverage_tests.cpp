#include "owo/wubi86/wubi86_encoder.h"
#include <algorithm>
#include <chrono>
#include <filesystem>
#include <fstream>
#include <iostream>
#include <map>
#include <set>
#include <stdexcept>
using namespace owo::wubi86;
#define CHECK(x) do { if (!(x)) throw std::runtime_error("line " + std::to_string(__LINE__) + ": " #x); } while(false)
static bool ordered(const Entry& a, const Entry& b) {
 if(a.frequency != b.frequency) return a.frequency > b.frequency;
 if(a.code != b.code) return a.code < b.code;
 return a.text < b.text;
}
int main(int argc,char** argv) {
 try {
  CHECK(argc==2);
  const auto start=std::chrono::steady_clock::now();
  const std::filesystem::path path(argv[1]);
  Wubi86Dictionary dictionary;const auto load_result=dictionary.load_file(path);CHECK(load_result.ok);
  std::cout<<"dictionary parse_us="<<load_result.parse_us<<" index_us="<<load_result.index_us<<'\n';
  const auto loaded=std::chrono::steady_clock::now();
  Wubi86CodeTable table(dictionary); Wubi86Encoder encoder(table);
  std::ifstream core(path.parent_path()/"gb2312.tsv",std::ios::binary);
  CHECK(core.good()); std::string line; std::size_t count=0; std::set<std::string> core_chars;
  while(std::getline(core,line)) {
   auto tab=line.find('\t'); CHECK(tab!=std::string::npos);
   const auto character=line.substr(0,tab); CHECK(split_utf8(character)->size()==1);
   CHECK(core_chars.insert(character).second);
   CHECK(!table.full_codes(character).empty()); CHECK(encoder.encode(character).ok); ++count;
  }
  CHECK(count==6763); CHECK(core_chars.contains("啊")); CHECK(core_chars.contains("齄"));
  // Explicit first full-code column, including three-key full codes and cross-source fixes.
  for (const auto& [text,code] : std::map<std::string,std::string>{
      {"好","vbg"},{"中","khk"},{"天","gdi"},{"人","wwww"},{"我","trnt"},
      {"发","ntcy"},{"靥","dddl"},{"式","aad"},{"麸","gqfw"},{"麴","fwwo"}}) {
   const auto codes=table.full_codes(text);CHECK(std::find(codes.begin(),codes.end(),code)!=codes.end());
  }
  CHECK(table.full_codes("\xee\xa0\x95").empty()); // Source PUA U+E815 is not a portable Han character.
  CHECK(table.short_codes("我")== (std::vector<std::string>{"q","trn"}));
  CHECK(table.full_codes("戈")== (std::vector<std::string>{"agnt","agny"}));
  CHECK(encoder.encode("你好").codes == std::vector<std::string>{"wqvb"});
  CHECK(encoder.encode("中华人民共和国").codes == std::vector<std::string>{"kwwl"});
  CHECK(encoder.encode("中国人").codes == std::vector<std::string>{"klww"});
  CHECK(encoder.encode("中国人民").codes == std::vector<std::string>{"klwn"});
  // Full graphical-root inventory, including source font slots that differ from Unicode glyphs.
  const std::map<char,std::size_t> expected_roots{{'g',6},{'f',11},{'d',13},{'s',4},{'a',16},
   {'h',11},{'j',10},{'k',3},{'l',12},{'m',10},{'t',9},{'r',10},{'e',16},{'w',6},{'q',15},
   {'y',10},{'u',13},{'i',12},{'o',6},{'p',5},{'n',24},{'b',12},{'v',8},{'c',6},{'x',9}};
  std::map<char,std::size_t> actual_roots;std::set<std::string_view> root_ids;
  for(const auto& root:Wubi86CodeTable::root_glyphs()) {
   CHECK(root_ids.insert(root.id).second);CHECK(!root.svg_path.empty());CHECK(root.source_slot.starts_with("U+"));
   const auto again=Wubi86CodeTable::root_glyph(root.id);CHECK(again && again->key==root.key);++actual_roots[root.key];
  }
  CHECK(actual_roots==expected_roots);CHECK(root_ids.size()==257);
  CHECK(Wubi86CodeTable::root_glyph("W86-g-02")->source_slot=="U+9752"); // 青字头, not the whole 青.
  CHECK(!Wubi86CodeTable::root_glyph("W86-z-01"));
  std::map<std::string,std::vector<Entry>> exact, prefixes;
  std::set<std::pair<std::string,std::string>> identities;
  std::size_t phrases=0, full_records=0, short_records=0;
  for(const auto& entry:dictionary.entries()) {
   CHECK(valid_code(entry.code));CHECK(identities.emplace(entry.code,entry.text).second);
   CHECK(entry.kind != CodeKind::unclassified);
   if(entry.kind==CodeKind::full) ++full_records;
   if(entry.kind==CodeKind::short_code) {
    ++short_records;CHECK(entry.code.size()<4);CHECK(!table.full_codes(entry.text).empty());
   }
   if(entry.kind==CodeKind::phrase) {
    const auto encoded=encoder.encode(entry.text);CHECK(encoded.ok);
    CHECK(std::find(encoded.codes.begin(),encoded.codes.end(),entry.code)!=encoded.codes.end());++phrases;
   }
   exact[entry.code].push_back(entry);
   for(std::size_t length=1;length<=entry.code.size();++length)prefixes[entry.code.substr(0,length)].push_back(entry);
  }
  CHECK(phrases==62586);CHECK(full_records==20925);CHECK(short_records==4183);CHECK(dictionary.size()==87694);
  for(auto& [code,expected]:exact) {
   std::sort(expected.begin(),expected.end(),ordered);const auto actual=dictionary.trie().exact(code);
   CHECK(actual.size()==expected.size());
   for(std::size_t i=0;i<actual.size();++i) CHECK(actual[i].text==expected[i].text && actual[i].frequency==expected[i].frequency);
  }
  for(auto& [prefix,expected]:prefixes) {
   std::sort(expected.begin(),expected.end(),ordered);const auto actual=dictionary.trie().prefix(prefix);
   CHECK(actual.size()==expected.size());
   for(std::size_t i=0;i<actual.size();++i)CHECK(actual[i].code==expected[i].code && actual[i].text==expected[i].text && actual[i].frequency==expected[i].frequency);
   const auto bounded=dictionary.trie().prefix(prefix,3);CHECK(bounded.size()==std::min<std::size_t>(3,expected.size()));
   for(std::size_t i=0;i<bounded.size();++i)CHECK(bounded[i].code==expected[i].code && bounded[i].text==expected[i].text);
  }
  const auto done=std::chrono::steady_clock::now();
  const auto load_ms=std::chrono::duration_cast<std::chrono::milliseconds>(loaded-start).count();
  const auto total_ms=std::chrono::duration_cast<std::chrono::milliseconds>(done-start).count();
  CHECK(load_ms<30000);CHECK(total_ms<120000); // Broad hang guards, not a fabricated throughput target.
  std::cout << "PASS core=" << count << " roots=" << root_ids.size() << " rows=" << dictionary.size()
   << " phrases=" << phrases << " exact_codes=" << exact.size() << " prefixes=" << prefixes.size()
   << " load_ms=" << load_ms << " total_ms=" << total_ms << '\n';
  return 0;
 } catch(const std::exception& e) {std::cerr << e.what() << '\n';return 1;}
}
