#include "../../plugins/wubi86/src/provider.h"
#include "owo/plugin/input_method_protocol.h"
#include <Windows.h>
#include <fstream>
#include <thread>
#include <barrier>
#include <iostream>
#include <stdexcept>
#define CHECK(x) do{if(!(x))throw std::runtime_error("line "+std::to_string(__LINE__)+": " #x);}while(false)
int main(int argc,char** argv){try{
 CHECK(argc==3);const auto root=std::filesystem::absolute(std::filesystem::path(argv[2])/("learning-"+std::to_string(GetCurrentProcessId())+"-"+std::to_string(GetTickCount64())));std::filesystem::create_directories(root);
 const auto path=root/"wubi-learning.tsv";const auto d=owo::wubi86::descriptor();
 owo::engine::InputMethodRequest r;r.request_id=1;r.session_id="learning";r.scheme_id=d.id;r.input="aaaa";r.caret_byte=4;
 std::string token,code,text,id;
 {owo::wubi86::Provider p(argv[1],path);auto v=p.handle(r,{});CHECK(v.candidates.size()>1);auto c=v.candidates[1];code=c.annotation;text=c.text;id=c.id;
  r.action=owo::engine::InputMethodAction::select;r.selected_candidate_id=id;++r.request_id;v=p.handle(r,{});CHECK(v.commit_id.size()==32);token=v.commit_id;
  CHECK(owo::wubi86::Learning(path).frequency(code,text)==0);CHECK(!std::filesystem::exists(path));
  auto legacy=d;legacy.commit_confirmation=false;
  const auto descriptors=owo::plugin::encode_input_method_descriptors(d.plugin_id,{d});std::vector<owo::engine::InputMethodDescriptor> described;
  CHECK(owo::plugin::decode_input_method_descriptors(d.plugin_id,descriptors,described).ok&&described[0].commit_confirmation);
  auto malformed=descriptors;malformed.back()=2;CHECK(!owo::plugin::decode_input_method_descriptors(d.plugin_id,malformed,described).ok);
  auto legacy_bytes=owo::plugin::encode_input_method_descriptors(d.plugin_id,{legacy});CHECK(legacy_bytes.find("OWCC")==std::string::npos);
  auto future_commit=r;future_commit.action=owo::engine::InputMethodAction::committed;future_commit.selected_candidate_id=token;
  CHECK(owo::plugin::encode_input_method_request(legacy,future_commit).empty());
  auto invalid_token=future_commit;invalid_token.selected_candidate_id="forged";CHECK(owo::plugin::encode_input_method_request(d,invalid_token).empty());
  const auto bytes=owo::plugin::encode_input_method_response(d,r,v);owo::engine::InputMethodResponse decoded;CHECK(owo::plugin::decode_input_method_response(d,r,bytes,decoded).ok);CHECK(decoded.commit_id==token);
  auto wrong=r;wrong.request_id++;wrong.action=owo::engine::InputMethodAction::committed;wrong.selected_candidate_id=token;wrong.session_id="wrong";CHECK(p.handle(wrong,{}).status==owo::engine::InputMethodStatus::error);
  r.action=owo::engine::InputMethodAction::committed;r.selected_candidate_id=token;++r.request_id;CHECK(p.handle(r,{}).status==owo::engine::InputMethodStatus::success);
  CHECK(owo::wubi86::Learning(path).frequency(code,text)==1);++r.request_id;CHECK(p.handle(r,{}).status==owo::engine::InputMethodStatus::success);CHECK(owo::wubi86::Learning(path).frequency(code,text)==1);
 }
 {owo::wubi86::Provider restored(argv[1],path);++r.request_id;CHECK(restored.handle(r,{}).status==owo::engine::InputMethodStatus::success);CHECK(owo::wubi86::Learning(path).frequency(code,text)==1);
  r.action=owo::engine::InputMethodAction::compose;r.selected_candidate_id.clear();++r.request_id;auto v=restored.handle(r,{});CHECK(v.candidates.front().id==id);
 }
 {std::ofstream corrupt(path,std::ios::binary);corrupt<<"broken";}
 {owo::wubi86::Provider recover(argv[1],path);++r.request_id;CHECK(!recover.handle(r,{}).candidates.empty());CHECK(owo::wubi86::Learning(path).frequency(code,text)==0);}
 bool quarantined=false;for(const auto& f:std::filesystem::directory_iterator(root))if(f.path().filename().string().find(".corrupt.")!=std::string::npos)quarantined=true;CHECK(quarantined);
 {std::ofstream saturation(path,std::ios::binary);saturation<<"OWO-WUBI-LEARN\t1\nentry\t"<<code<<'\t'<<text<<"\t1000000\n";}
 {owo::wubi86::Learning store(path);CHECK(store.commit(std::string(32,'a'),code,text,"binding"));CHECK(store.frequency(code,text)==1000000);CHECK(!store.commit(std::string(32,'a'),code,text,"other"));}
 const auto concurrent=root/"concurrent.tsv";
 {std::barrier gate(2);bool one=false,two=false;
  std::thread a([&]{owo::wubi86::Learning store(concurrent);gate.arrive_and_wait();one=store.commit(std::string(32,'b'),"wqiy","你","same");});
  std::thread b([&]{owo::wubi86::Learning store(concurrent);gate.arrive_and_wait();two=store.commit(std::string(32,'c'),"wqiy","你","same");});
  a.join();b.join();CHECK(one&&two);CHECK(owo::wubi86::Learning(concurrent).frequency("wqiy","你")==2);
 }
 {owo::wubi86::Learning store(concurrent);CHECK(store.commit(std::string(32,'b'),"wqiy","你","same"));CHECK(store.frequency("wqiy","你")==2);}
 {
  HANDLE locked=CreateFileW(concurrent.c_str(),GENERIC_READ,0,nullptr,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,nullptr);CHECK(locked!=INVALID_HANDLE_VALUE);
  owo::wubi86::Learning blocked(concurrent);CHECK(!blocked.commit(std::string(32,'d'),"wqiy","你","same"));CloseHandle(locked);
  CHECK(owo::wubi86::Learning(concurrent).frequency("wqiy","你")==2);
 }
 const auto corrupt_locked=root/"corrupt-locked.tsv";{std::ofstream out(corrupt_locked);out<<"broken";}
 {
  HANDLE locked=CreateFileW(corrupt_locked.c_str(),GENERIC_READ,FILE_SHARE_READ,nullptr,OPEN_EXISTING,FILE_ATTRIBUTE_NORMAL,nullptr);CHECK(locked!=INVALID_HANDLE_VALUE);
  owo::wubi86::Learning blocked(corrupt_locked);CHECK(!blocked.commit(std::string(32,'e'),"wqiy","你","same"));CloseHandle(locked);
  std::ifstream verify(corrupt_locked);std::string value;verify>>value;CHECK(value=="broken");
 }
 { owo::wubi86::Learning bounded(root/"oversize.tsv"); CHECK(!bounded.commit(std::string(32,'f'),"a",std::string(2*1024*1024,'x'),"bound")); CHECK(bounded.frequency("a",std::string(2*1024*1024,'x'))==0); }
 std::cout<<"PASS unreadable snapshots and quarantine failure preserve originals\n";
 std::cout<<"PASS concurrent atomic writers and replay\n";
 std::cout<<"PASS post-commit only, wire token, wrong session, idempotence, restart, ranking, quarantine, saturation\n";return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}
