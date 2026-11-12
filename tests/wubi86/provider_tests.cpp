#include "../../plugins/wubi86/src/provider.h"
#include <iostream>
#include <stdexcept>
#include <fstream>
#include <chrono>
#include <algorithm>
#define CHECK(x) do {if(!(x))throw std::runtime_error(#x);}while(false)
int main(int argc,char** argv) {try{
 CHECK(argc==2);owo::wubi86::Provider provider(argv[1]);auto d=owo::wubi86::descriptor();
 owo::engine::InputMethodRequest r;r.request_id=1;r.session_id="test";r.scheme_id=d.id;r.input="wqiy";r.caret_byte=4;r.page_size=7;
 auto out=provider.handle(r,{});CHECK(owo::engine::validate_input_method_response(d,r,out).ok);CHECK(!out.candidates.empty());CHECK(out.candidates[0].text=="你");CHECK(out.candidates[0].consumed_ranges[0].end==4);
 auto id=out.candidates[0].id;r.request_id++;r.action=owo::engine::InputMethodAction::select;r.selected_candidate_id=id;CHECK(provider.handle(r,{}).candidates[0].id==id);
 r.request_id++;r.session_id="another";CHECK(provider.handle(r,{}).status==owo::engine::InputMethodStatus::error);
 r.action=owo::engine::InputMethodAction::compose;r.selected_candidate_id.clear();r.input="khlg";r.caret_byte=4;CHECK(provider.handle(r,{}).candidates[0].text=="中国");
 r.request_id++;r.input="a";r.caret_byte=1;out=provider.handle(r,{});CHECK(out.candidates.size()==7);CHECK(out.has_more);auto first=out.candidates[0].id;
 r.request_id++;r.page=1;out=provider.handle(r,{});CHECK(!out.candidates.empty());CHECK(out.candidates[0].id!=first);
 r.request_id++;r.page=0;CHECK(provider.handle(r,{}).candidates[0].id==first);
 r.request_id++;r.input.clear();r.caret_byte=0;CHECK(provider.handle(r,{}).candidates.empty());
 r.request_id++;r.input="z";r.caret_byte=1;CHECK(provider.handle(r,{}).status==owo::engine::InputMethodStatus::error);
 r.request_id++;r.input="yyyyy";r.caret_byte=5;CHECK(provider.handle(r,{}).status==owo::engine::InputMethodStatus::error);
 r.input="a";r.caret_byte=1;std::stop_source stop;stop.request_stop();CHECK(provider.handle(r,stop.get_token()).status==owo::engine::InputMethodStatus::cancelled);
 const auto fixture=std::filesystem::temp_directory_path()/("owo-wubi-scheme-"+std::to_string(std::chrono::steady_clock::now().time_since_epoch().count()));
 const auto scheme=fixture/"input-schemes"/"wubi86";std::filesystem::create_directories(scheme);
 {std::ofstream user(scheme/"user-lexicon.tsv",std::ios::binary);user<<"OWO-WUBI86\t1\nversion\tfixture\nsource\tfixture\n...\nqxyy\t专用词条\t900\tphrase\n";}
 {std::ofstream settings(fixture/"settings.json",std::ios::binary);settings<<"{\"show_prefix_candidates\": false}";}
 owo::wubi86::Provider scoped(argv[1],{},scheme);
 r.input="qxyy";r.caret_byte=4;r.request_id++;auto custom=scoped.handle(r,{});
 CHECK(!custom.candidates.empty()&&custom.candidates[0].text=="专用词条");
 r.input="qxy";r.caret_byte=3;r.request_id++;auto no_prefix=scoped.handle(r,{});
 CHECK(std::none_of(no_prefix.candidates.begin(),no_prefix.candidates.end(),[](const auto& value){return value.text=="专用词条";}));
 auto host=provider.handle(r,{});
 CHECK(std::none_of(host.candidates.begin(),host.candidates.end(),[](const auto& value){return value.text=="专用词条";}));
 std::filesystem::remove(scheme/"user-lexicon.tsv");std::filesystem::remove(fixture/"settings.json");
 std::filesystem::remove(scheme);std::filesystem::remove(scheme.parent_path());std::filesystem::remove(fixture);
 std::cout<<"PASS production provider queries, paging, identity, selection, validation, cancellation\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what();return 1;}}
