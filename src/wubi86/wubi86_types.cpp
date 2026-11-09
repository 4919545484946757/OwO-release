#include "owo/wubi86/wubi86_types.h"
namespace owo::wubi86 {
bool valid_code(std::string_view code) noexcept {
 if(code.empty() || code.size()>4) return false;
 for(char c:code) if(c<'a'||c>'y') return false;
 return true;
}
namespace {
std::optional<std::size_t> walk_utf8(std::string_view text,std::vector<std::string>* out) {
 std::size_t count=0;
 for(std::size_t i=0;i<text.size();) {
  auto first=static_cast<unsigned char>(text[i]);
  std::size_t length=0; std::uint32_t scalar=0, minimum=0;
  if(first<0x80) {length=1;scalar=first;}
  else if(first>=0xc2&&first<=0xdf) {length=2;scalar=first&0x1f;minimum=0x80;}
  else if(first>=0xe0&&first<=0xef) {length=3;scalar=first&0x0f;minimum=0x800;}
  else if(first>=0xf0&&first<=0xf4) {length=4;scalar=first&7;minimum=0x10000;}
  else return std::nullopt;
  if(length>text.size()-i) return std::nullopt;
  for(std::size_t j=1;j<length;++j) {auto b=static_cast<unsigned char>(text[i+j]);if((b&0xc0)!=0x80)return std::nullopt;scalar=(scalar<<6)|(b&0x3f);}
  if(scalar<minimum||scalar>0x10ffff||(scalar>=0xd800&&scalar<=0xdfff)||scalar<0x20||(scalar>=0x7f&&scalar<=0x9f)) return std::nullopt;
  if(out)out->emplace_back(text.substr(i,length));++count;i+=length;
 }
 return count;
}
}
std::optional<std::size_t> utf8_scalar_count(std::string_view text) {return walk_utf8(text,nullptr);}
std::optional<std::vector<std::string>> split_utf8(std::string_view text) {
 std::vector<std::string> out;if(!walk_utf8(text,&out))return std::nullopt;return out;
}
}
