#include "owo/wubi86/wubi86_encoder.h"
#include <set>
namespace owo::wubi86 {
EncodeResult Wubi86Encoder::encode(std::string_view text,std::size_t max_variants) const {
 const auto characters=split_utf8(text);
 if(!characters||characters->empty()||max_variants==0)return {false,{},"empty/invalid UTF-8 input or zero variant limit"};
 if(characters->size()==1) {auto codes=table_.full_codes(text);if(codes.empty())return {false,{},"no verified full code"};if(codes.size()>max_variants)return {false,{},"variant limit exceeded"};return {true,std::move(codes),{}};}
 // Validate every character, including middle characters not selected by the 4+ rule.
 for(const auto& character:*characters)if(table_.full_codes(character).empty())return {false,{},"character has no verified full code: "+character};
 const auto count=characters->size();
 std::vector<std::pair<std::size_t,std::size_t>> parts;
 if(count==2)parts={{0,2},{1,2}};
 else if(count==3)parts={{0,1},{1,1},{2,2}};
 else parts={{0,1},{1,1},{2,1},{count-1,1}};
 std::set<std::string> results{""};
 for(auto [index,take]:parts) {
  std::set<std::string> fragments,next;
  for(const auto& code:table_.full_codes((*characters)[index])) {
   if(code.size()<take)return {false,{},"full code shorter than required phrase fragment"};
   fragments.insert(code.substr(0,take));
  }
  for(const auto& prefix:results)for(const auto& fragment:fragments) {next.insert(prefix+fragment);if(next.size()>max_variants)return {false,{},"variant limit exceeded"};}
  results=std::move(next);
 }
 return {true,{results.begin(),results.end()},{}};
}
}
