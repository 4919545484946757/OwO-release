#include "owo/wubi86/wubi86_trie.h"
#include <algorithm>
namespace owo::wubi86 {
namespace {
bool order(const Entry& a,const Entry& b) {
 if(a.frequency!=b.frequency)return a.frequency>b.frequency;
 if(a.code!=b.code)return a.code<b.code;
 return a.text<b.text;
}
}
Wubi86Trie::Wubi86Trie() : nodes_(1) {}
bool Wubi86Trie::insert(const Entry& entry) {
 const auto text=utf8_scalar_count(entry.text);
 if(!valid_code(entry.code)||!text||*text==0)return false;
 return insert_validated(entry);
}
bool Wubi86Trie::insert_validated(const Entry& entry) {
 std::size_t node=0;
 for(char c:entry.code) {
  const auto index=static_cast<std::size_t>(c-'a');
  if(!nodes_[node].children[index]) {const auto next=nodes_.size();nodes_[node].children[index]=next;nodes_.emplace_back();}
  node=nodes_[node].children[index];
 }
 auto& entries=nodes_[node].entries;
 for(auto& old:entries) if(old.text==entry.text) {old.frequency=std::max(old.frequency,entry.frequency);return false;}
 entries.push_back(entry);return true;
}
std::size_t Wubi86Trie::find(std::string_view code) const {
 if(!valid_code(code))return nodes_.size();
 std::size_t node=0;
 for(char c:code) {node=nodes_[node].children[static_cast<std::size_t>(c-'a')];if(!node)return nodes_.size();}
 return node;
}
std::vector<Entry> Wubi86Trie::exact(std::string_view code) const {
 const auto node=find(code);if(node==nodes_.size())return {};
 auto result=nodes_[node].entries;std::sort(result.begin(),result.end(),order);return result;
}
std::vector<Entry> Wubi86Trie::prefix(std::string_view code,std::size_t limit) const {
 const auto root=find(code);if(root==nodes_.size()||limit==0)return {};
 std::vector<Entry> result;std::vector<std::size_t> pending{root};
 while(!pending.empty()) {const auto node=pending.back();pending.pop_back();
  result.insert(result.end(),nodes_[node].entries.begin(),nodes_[node].entries.end());
  for(auto child:nodes_[node].children)if(child)pending.push_back(child);
 }
 std::sort(result.begin(),result.end(),order);if(result.size()>limit)result.resize(limit);return result;
}
}
