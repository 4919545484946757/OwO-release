#include "provider.h"
#include <algorithm>
#include <stdexcept>
#include <numeric>
#include <Windows.h>
#include <bcrypt.h>
namespace owo::wubi86 {
engine::InputMethodDescriptor descriptor() {
 engine::InputMethodDescriptor d; d.plugin_id="owo.plugin.wubi86";d.id=d.plugin_id+"/wubi86";
 d.name="五笔86";d.language_tag="zh-Hans";d.kind=engine::InputMethodKind::table;
 d.alphabet=U"abcdefghijklmnopqrstuvwxy";d.max_input_bytes=4;d.max_candidates=128;d.commit_confirmation=true;return d;
}
Provider::Provider(const std::filesystem::path& path,const std::filesystem::path& learning) : learning_(learning) {
 auto loaded=dictionary_.load_file(path);if(!loaded.ok)throw std::runtime_error(loaded.error);
}
engine::InputMethodResponse Provider::handle(const engine::InputMethodRequest& r,std::stop_token stop) {
 std::lock_guard lock(mutex_);
 engine::InputMethodResponse out;out.request_id=r.request_id;out.session_id=r.session_id;
 out.scheme_id=r.scheme_id;out.page=r.page;out.preview=r.input;
 auto error=[&](std::string message) {issued_.clear();out.candidates.clear();out.has_more=false;out.status=engine::InputMethodStatus::error;out.diagnostic=std::move(message);return out;};
 if(!engine::validate_input_method_request(descriptor(),r).ok)return error("Invalid Wubi request");
 if(stop.stop_requested()) {out.status=engine::InputMethodStatus::cancelled;return out;}
 if(r.action==engine::InputMethodAction::committed) {
  std::string raw_binding=r.session_id;raw_binding.push_back('\0');raw_binding+=r.input;raw_binding.push_back('\0');raw_binding+=std::to_string(r.page);
  std::string binding;constexpr char hex[]="0123456789abcdef";for(unsigned char c:raw_binding){binding+=hex[c>>4];binding+=hex[c&15];}
  if(learning_.contains(r.selected_candidate_id)){if(!learning_.matches(r.selected_candidate_id,binding))return error("Commit binding mismatch");return out;}
  if(!prepared_||prepared_->token!=r.selected_candidate_id||prepared_->session!=r.session_id||prepared_->input!=r.input||prepared_->page!=r.page)return error("Unknown commit transaction");
  if(!learning_.commit(prepared_->token,prepared_->code,prepared_->text,binding))return error("Learning persistence unavailable");
  prepared_.reset();sequence_=r.request_id;return out;
 }
 if(session_==r.session_id && r.request_id<=sequence_)return error("Stale Wubi request");
 prepared_.reset();
 if(r.action==engine::InputMethodAction::select) {
  if(session_!=r.session_id||input_!=r.input||page_!=r.page)return error("Stale selection");
  auto found=std::find_if(issued_.begin(),issued_.end(),[&](const auto& c){return c.id==r.selected_candidate_id;});
  if(found==issued_.end())return error("Unknown candidate");
  BYTE nonce[16];if(BCryptGenRandom(nullptr,nonce,sizeof nonce,BCRYPT_USE_SYSTEM_PREFERRED_RNG)!=0)return error("Commit identity unavailable");
  std::string token;constexpr char hex[]="0123456789abcdef";for(auto c:nonce){token+=hex[c>>4];token+=hex[c&15];}
  prepared_=Commit{token,r.session_id,r.input,found->annotation,found->text,r.page};
  out.commit_id=token;out.candidates.push_back(*found);issued_.clear();sequence_=r.request_id;return out;
 }
 session_=r.session_id;input_=r.input;page_=r.page;sequence_=r.request_id;issued_.clear();
 if(r.action==engine::InputMethodAction::cancel) {out.status=engine::InputMethodStatus::cancelled;return out;}
 if(r.input.empty())return out;
 auto matches=dictionary_.trie().prefix(r.input);
 // Read each learned frequency once, rather than allocating map keys in every sort comparison.
 std::vector<std::size_t> ranked(matches.size());std::iota(ranked.begin(),ranked.end(),0);
 std::vector<std::uint32_t> learned;learned.reserve(matches.size());bool any_learned=false;
 for(const auto& entry:matches){const auto n=learning_.frequency(entry.code,entry.text);learned.push_back(n);any_learned|=n!=0;}
 if(any_learned)std::stable_sort(ranked.begin(),ranked.end(),[&](auto a,auto b){
  if((matches[a].code==r.input)!=(matches[b].code==r.input))return matches[a].code==r.input;
  return learned[a]>learned[b];
 });
 else std::stable_partition(ranked.begin(),ranked.end(),[&](auto i){return matches[i].code==r.input;});
 const auto begin=r.page*r.page_size;
 for(std::size_t i=begin;i<matches.size();++i) {
  if(stop.stop_requested()) {out.candidates.clear();out.status=engine::InputMethodStatus::cancelled;return out;}
  if(out.candidates.size()==r.page_size) {out.has_more=true;break;}
  const auto& entry=matches[ranked[i]];std::string id=entry.code+":";
  constexpr char hex[]="0123456789abcdef";
  for(unsigned char c:entry.text){id+=hex[c>>4];id+=hex[c&15];}
  if(id.size()>128)return error("Candidate identity exceeds protocol limit");
  out.candidates.push_back({std::move(id),entry.text,entry.code,{{0,r.input.size()}}});
 }
 issued_=out.candidates;return out;
}
}
