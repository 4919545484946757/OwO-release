#include "../../plugins/wubi86/src/provider.h"
#include <iostream>
#include <stdexcept>
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
 std::cout<<"PASS production provider queries, paging, identity, selection, validation, cancellation\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what();return 1;}}
