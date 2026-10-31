#include "owo/wubi86/wubi86_dictionary.h"
#include <charconv>
#include <array>
#include <chrono>
#include <fstream>
#include <unordered_map>
#include <tuple>
namespace owo::wubi86 {
namespace {
std::optional<std::array<std::string_view,4>> fields(std::string_view line) {
 std::array<std::string_view,4> out;std::size_t start=0;
 for(std::size_t i=0;i<4;++i) {
  const auto end=line.find('\t',start);
  if((i<3 && end==std::string_view::npos)||(i==3 && end!=std::string_view::npos))return std::nullopt;
  out[i]=line.substr(start,end==std::string_view::npos?end:end-start);
  start=end==std::string_view::npos?line.size():end+1;
 }
 return out;
}
}
LoadResult Wubi86Dictionary::load_file(const std::filesystem::path& path) {
 std::ifstream input(path,std::ios::binary);if(!input)return {false,"cannot open dictionary",0,0};return load(input);
}
LoadResult Wubi86Dictionary::load(std::istream& input) {
 const auto started=std::chrono::steady_clock::now();
 Wubi86Dictionary next;std::string line;std::size_t number=0,duplicates=0;
 auto read=[&] {if(!std::getline(input,line))return false;++number;if(!line.empty()&&line.back()=='\r')line.pop_back();return true;};
 auto fail=[&](std::string message) {return LoadResult{false,std::move(message),number,duplicates};};
 if(!read()||line!="OWO-WUBI86\t1")return fail("unsupported format/version");
 if(!read()||!line.starts_with("version\t")||line.size()==8)return fail("missing data version");
 next.version_=line.substr(8);
 if(!read()||!line.starts_with("source\t")||line.size()==7)return fail("missing source");
 next.source_=line.substr(7);
 if(!split_utf8(next.version_)||!split_utf8(next.source_))return fail("invalid metadata UTF-8");
 if(!read()||line!="...")return fail("missing data separator");
 std::unordered_map<std::string,std::size_t> identities;
 while(read()) {
  auto parsed_fields=fields(line);if(!parsed_fields)return fail("expected code/text/frequency/kind");
  const auto& f=*parsed_fields;
  const auto scalars=utf8_scalar_count(f[1]);if(!valid_code(f[0])||!scalars||*scalars==0)return fail("invalid code or UTF-8 text");
  std::uint32_t frequency=0;auto parsed=std::from_chars(f[2].data(),f[2].data()+f[2].size(),frequency);
  if(f[2].empty()||parsed.ec!=std::errc{}||parsed.ptr!=f[2].data()+f[2].size())return fail("invalid frequency");
  CodeKind kind;
  if(f[3]=="full")kind=CodeKind::full;
  else if(f[3]=="short")kind=CodeKind::short_code;
  else if(f[3]=="unclassified")kind=CodeKind::unclassified;
  else if(f[3]=="phrase")kind=CodeKind::phrase;
  else return fail("invalid kind");
  if((kind==CodeKind::full||kind==CodeKind::short_code)&&*scalars!=1)return fail("character kind requires one scalar");
  if(kind==CodeKind::short_code&&f[0].size()>3)return fail("short code too long");
  if(kind==CodeKind::phrase&&*scalars<2)return fail("phrase requires multiple scalars");
  Entry entry{std::string(f[0]),std::string(f[1]),frequency,kind};
  std::string identity;identity.reserve(f[0].size()+1+f[1].size());identity.append(f[0]);identity.push_back('\t');identity.append(f[1]);
  auto [it,inserted]=identities.emplace(std::move(identity),next.entries_.size());
  if(!inserted) {auto& old=next.entries_[it->second];if(old.kind!=kind)return fail("conflicting duplicate kind");old.frequency=std::max(old.frequency,frequency);++duplicates;}
  else next.entries_.push_back(std::move(entry));
 }
 if(input.bad()||(!input.eof()&&input.fail()))return fail("dictionary read error");
 if(next.entries_.empty())return fail("empty dictionary");
 // Parsing above already validates code and text. Reserve the common-case node
 // count to avoid relocating tens of thousands of Debug iterator containers.
 const auto indexed=std::chrono::steady_clock::now();
 next.trie_.nodes_.reserve(next.entries_.size()+1);
 for(const auto& entry:next.entries_)next.trie_.insert_validated(entry);
 const auto finished=std::chrono::steady_clock::now();
 *this=std::move(next);return {true,{},number,duplicates,
  static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(indexed-started).count()),
  static_cast<std::uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(finished-indexed).count())};
}
}
