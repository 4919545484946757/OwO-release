#include "owo/core/input_method_runtime.h"
#include "owo/engine/binary_lexicon.h"
#include "owo/plugin/plugin_store.h"
#include "owo/plugin/plugin_authorization_store.h"
#include "../../plugins/wubi86/src/provider.h"
#include "owo/ipc/input_method_client.h"
#include "owo/tsf/input_method_session.h"
#include <Windows.h>
#include <algorithm>
#include <fstream>
#include <iostream>
#include <stdexcept>
#define CHECK(x) do {if(!(x))throw std::runtime_error(#x);}while(false)
int main(int argc,char** argv) {try {
 CHECK(argc==3 || argc==4);
 const auto root=std::filesystem::absolute(std::filesystem::path(argv[1])/("wubi-process-"+std::to_string(GetCurrentProcessId())+"-"+std::to_string(GetTickCount64())));
 CHECK(!std::filesystem::exists(root));std::filesystem::create_directories(root.parent_path());
 auto initialized=owo::plugin::initialize_plugin_store(root);if(!initialized.ok)std::cerr<<initialized.diagnostic<<'\n';CHECK(initialized.ok);
 const auto stage=root/"staging"/".wubi";
 std::filesystem::copy(argv[2],stage,std::filesystem::copy_options::recursive);
 CHECK(owo::plugin::publish_staged_plugin(root,stage,std::string(64,'a'),std::string(64,'b')).ok);
 const auto d=owo::wubi86::descriptor();
 CHECK(!owo::core::discover_plugin_input_methods(root,d.plugin_id,std::chrono::seconds(5)).validation.ok);
 const auto active=owo::plugin::query_active_plugin_version(root,d.plugin_id);CHECK(active.ok);
 const auto grant=owo::plugin::make_plugin_authorization(active.manifest,active.inventory_sha256,active.publisher_certificate_sha256,{"input.scheme"});
 CHECK(grant.ok);CHECK(owo::plugin::save_plugin_authorization(root,grant.value).ok);
 const auto found=owo::core::discover_plugin_input_methods(root,d.plugin_id,std::chrono::seconds(5));
 if(!found.validation.ok)std::cerr<<found.validation.diagnostic<<'\n';
 CHECK(found.validation.ok);CHECK(found.descriptors.size()==1);CHECK(found.descriptors[0].id==d.id);
 owo::engine::InputMethodRegistry registry;
 CHECK(owo::core::register_plugin_input_method(registry,root,d,std::chrono::seconds(5)).ok);
 owo::engine::InputMethodRequest r;r.scheme_id=d.id;r.session_id="process";r.request_id=1;r.input="wqiy";r.caret_byte=4;
 auto out=registry.dispatch(r);if(out.status!=owo::engine::InputMethodStatus::success)std::cerr<<out.diagnostic<<'\n';
 CHECK(out.status==owo::engine::InputMethodStatus::success);CHECK(out.candidates.front().text=="你");
 auto id=out.candidates.front().id;r.action=owo::engine::InputMethodAction::select;r.selected_candidate_id=id;++r.request_id;
 CHECK(registry.dispatch(r).candidates.front().id==id);
 ++r.request_id;CHECK(registry.dispatch(r).status==owo::engine::InputMethodStatus::error);
 r.action=owo::engine::InputMethodAction::compose;r.selected_candidate_id.clear();r.input="a";r.caret_byte=1;r.page_size=2;++r.request_id;
 out=registry.dispatch(r);CHECK(out.candidates.size()==2);CHECK(out.has_more);auto first=out.candidates[0].id;
 ++r.request_id;r.page=1;CHECK(registry.dispatch(r).candidates[0].id!=first);
 ++r.request_id;r.input="z";CHECK(registry.dispatch(r).status==owo::engine::InputMethodStatus::error);
 const auto revoked=owo::plugin::make_plugin_authorization(active.manifest,active.inventory_sha256,active.publisher_certificate_sha256,{});
 CHECK(revoked.ok);CHECK(owo::plugin::save_plugin_authorization(root,revoked.value).ok);
 ++r.request_id;r.input="a";r.page=0;CHECK(registry.dispatch(r).status!=owo::engine::InputMethodStatus::success);
 registry.remove_plugin(d.plugin_id);CHECK(owo::plugin::save_plugin_authorization(root,grant.value).ok);
 CHECK(owo::core::register_plugin_input_method(registry,root,d,std::chrono::seconds(5)).ok);
 ++r.request_id;CHECK(!registry.dispatch(r).candidates.empty());

 {
  const auto denied_pipe=std::wstring(LR"(\\.\pipe\OwO.Test.Wubi.Denied.)")+std::to_wstring(GetCurrentProcessId());
  owo::ipc::InputMethodPipeServer denied_server(root,denied_pipe,[](std::uint32_t){return owo::ipc::InputMethodFocus{};});
  CHECK(denied_server.started());owo::ipc::InputMethodAsyncClient denied_client(denied_pipe);
  auto denied=denied_client.open(1,d.plugin_id,d.id,"denied-focus",false).get();
  CHECK(!denied.status);CHECK(denied.status.message=="Control busy or unsafe focus");
 }
 {
  const auto pipe=std::wstring(LR"(\\.\pipe\OwO.Test.Wubi.)")+std::to_wstring(GetCurrentProcessId());
  owo::ipc::InputMethodPipeServer server(root,pipe,[](std::uint32_t){return owo::ipc::InputMethodFocus{1,false};});
  CHECK(server.started());owo::ipc::InputMethodAsyncClient client(pipe);
  auto get=[](std::future<owo::ipc::InputMethodClientResult> f){CHECK(f.wait_for(std::chrono::seconds(8))==std::future_status::ready);return f.get();};
  auto opened=get(client.open(1,d.plugin_id,d.id,"ipc-session",false));
  if(!opened.status)std::cerr<<"IPC open failed\n";
  owo::tsf::InputMethodDocumentSession document;CHECK(document.enter(1));CHECK(document.bind(opened,"ipc-session"));
  auto& composition=document.composition();CHECK(composition.insert("a"));
  auto request=composition.compose_request(1,0,7);CHECK(request);
  const auto cold_start=std::chrono::steady_clock::now();
  auto candidates=get(client.submit(1,*request,false));
  const auto cold_ms=std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now()-cold_start).count();
  std::cout<<"cold IPC roundtrip ms="<<cold_ms<<'\n';CHECK(cold_ms<1000);
  CHECK(document.apply(candidates).accepted);
  CHECK(candidates.response);if(candidates.response->candidates.empty())std::cerr<<"IPC candidates status="<<static_cast<int>(candidates.response->status)<<" diagnostic="<<candidates.response->diagnostic<<'\n';
  CHECK(!candidates.response->candidates.empty());
  CHECK(composition.backspace());CHECK(composition.insert("wqiy"));
  auto full_request=composition.compose_request(2,0,7);CHECK(full_request);
  candidates=get(client.submit(1,*full_request,false));CHECK(document.apply(candidates).accepted);
  CHECK(candidates.response && !candidates.response->candidates.empty());CHECK(candidates.response->candidates.front().text=="你");
  auto selected=composition.select_request(3,candidates.response->candidates.front().id);CHECK(selected);
  auto acknowledgement=get(client.submit(1,*selected,false));auto prepared=document.apply(acknowledgement);
  CHECK(prepared.accepted && prepared.commit && prepared.commit->text=="你");
  CHECK(acknowledgement.response && acknowledgement.response->commit_id.size()==32);
  auto confirmation=*selected;confirmation.action=owo::engine::InputMethodAction::committed;
  confirmation.selected_candidate_id=acknowledgement.response->commit_id;confirmation.request_id=4;
  auto learned=get(client.submit(1,confirmation,false));CHECK(learned.status && learned.response && learned.response->status==owo::engine::InputMethodStatus::success);
  confirmation.request_id=5;auto duplicate=get(client.submit(1,confirmation,false));CHECK(duplicate.status && duplicate.response && duplicate.response->status==owo::engine::InputMethodStatus::success);
  CHECK(document.finish_commit(1,true));CHECK(composition.input().empty());
  CHECK(!document.finish_commit(1,true));CHECK(!document.apply(acknowledgement).accepted);
  // A five-letter spelling must reach Core even when it has no production pinyin lexicon.
  CHECK(composition.insert("nihao"));
  auto spelling=composition.compose_request(6,0,7);CHECK(spelling);
  auto missing=get(client.submit(1,*spelling,false));CHECK(document.apply(missing).accepted);
  CHECK(missing.response && missing.response->candidates.empty());
  CHECK(missing.response->diagnostic.find("lexicon")!=std::string::npos);
  for(int i=0;i<5;++i)CHECK(composition.backspace());
  CHECK(composition.insert("a"));CHECK(composition.backspace());CHECK(composition.input().empty());
  document.invalidate();CHECK(!document.apply(candidates).accepted);CHECK(get(client.close()).status);
 }
 {
  using namespace owo::engine;
  const bool production=argc==4;
  BinaryLexicon binary;
  if(production) {
   const auto loaded=binary.load(argv[3]);if(!loaded.success)std::cerr<<loaded.error<<'\n';
   CHECK(loaded.success);
  }
  // Small fixture exercises routing in regular CTest. Only the explicit
  // production invocation checks the real nihao and zhongguo results.
  const MemoryLexicon fixture({
   {{"zhang","san"},"张三",1000},{{"zhang","san"},"章三",100},
   {{"zhang"},"张",500},{{"san"},"三",500},{{"ni"},"你",500}});
  const Lexicon* lexicon=production ? static_cast<const Lexicon*>(&binary) : &fixture;
  const auto pipe=std::wstring(LR"(\\.\pipe\OwO.Test.Wubi.Pinyin.)")+std::to_wstring(GetCurrentProcessId());
  owo::ipc::InputMethodPipeServer server(root,pipe,
      [](std::uint32_t){return owo::ipc::InputMethodFocus{1,false};},lexicon);
  CHECK(server.started());
  owo::ipc::InputMethodAsyncClient client(pipe);
  auto opened=client.open(2,d.plugin_id,d.id,"fallback",false).get();
  owo::tsf::InputMethodDocumentSession document;
  CHECK(document.enter(2));CHECK(document.bind(opened,"fallback"));
  auto& composition=document.composition();
  std::uint64_t sequence=0;
  auto submit=[&](const InputMethodRequest& request) {
   auto result=client.submit(2,request,false).get();
   CHECK(result.status && result.response);
   return result;
  };
  auto clear=[&] {while(!composition.input().empty())CHECK(composition.backspace());};
  auto query=[&](std::size_t page=0,std::size_t size=7) {
   auto request=composition.compose_request(++sequence,page,size);CHECK(request);
   auto result=submit(*request);
   CHECK(document.apply(result).accepted);
   CHECK(result.response->status==InputMethodStatus::success);
   return result;
  };
  auto learning=[&] {
   for(const auto& file:std::filesystem::recursive_directory_iterator(root))
    if(file.path().filename()=="wubi86-learning.v1.tsv"){
     std::ifstream stream(file.path(),std::ios::binary);
     return std::string(std::istreambuf_iterator<char>(stream),{});
    }
   return std::string{};
  };
  const auto learned_before=learning();CHECK(!learned_before.empty());
  CHECK(composition.insert("ni"));
  auto result=query();
  CHECK(!result.response->candidates.empty());
  CHECK(result.response->candidates.front().text=="悄");
  CHECK(result.response->candidates.front().id.starts_with("ni:"));
  CHECK(query(1000000).response->candidates.empty()); // An exhausted Wubi page cannot start pinyin.
  clear();
  CHECK(composition.insert("zzzz"));
  CHECK(query().response->candidates.empty());
  clear();
  CHECK(composition.insert("zh"));
  auto prefix=query();
  CHECK(!prefix.response->candidates.empty());
  CHECK(prefix.response->candidates.front().id.starts_with("pinyin:"));
  clear();
  const std::string input=production ? "nihao" : "zhangsan";
  const std::string expected=production ? "你好" : "张三";
  CHECK(composition.insert(input));
  result=query(0,1);
  CHECK(result.response->candidates.size()==1 && result.response->has_more);
  const auto first=result.response->candidates.front();
  CHECK(first.text==expected && first.id.starts_with("pinyin:"));
  CHECK(first.consumed_ranges.size()==1 && first.consumed_ranges.front().end==input.size());
  auto next=query(1,1);
  CHECK(next.response->candidates.size()==1 && next.response->candidates.front().text!=expected);
  CHECK(!composition.select_request(sequence+1,first.id));
  CHECK(composition.backspace());CHECK(!document.apply(next).accepted);
  CHECK(!query().response->candidates.empty());
  CHECK(composition.insert(input.substr(input.size()-1)));
  result=query();
  CHECK(!result.response->candidates.empty() && result.response->candidates.front().text==expected);
  auto selected=composition.select_request(++sequence,result.response->candidates.front().id);CHECK(selected);
  auto cancel=*selected;
  cancel.action=InputMethodAction::cancel;
  cancel.selected_candidate_id.clear();
  cancel.request_id=++sequence;
  CHECK(submit(cancel).response->status==InputMethodStatus::cancelled);
  selected->request_id=++sequence;
  CHECK(submit(*selected).response->status==InputMethodStatus::error);
  CHECK(learning()==learned_before);
  result=query();
  selected=composition.select_request(++sequence,result.response->candidates.front().id);CHECK(selected);
  auto response=submit(*selected);auto prepared=document.apply(response);
  CHECK(prepared.accepted && prepared.commit && prepared.commit->text==expected);
  CHECK(learning()==learned_before);
  CHECK(document.finish_commit(2,true));CHECK(composition.input().empty());
  selected->action=InputMethodAction::committed;
  selected->selected_candidate_id=response.response->commit_id;
  selected->request_id=++sequence;
  CHECK(submit(*selected).response->status==InputMethodStatus::success);
  selected->request_id=++sequence;
  CHECK(submit(*selected).response->status==InputMethodStatus::success);
  CHECK(learning()==learned_before);
  if(production) {
   CHECK(composition.insert("zhongguo"));
   auto country=query();
   CHECK(!country.response->candidates.empty() && country.response->candidates.front().text=="中国");
   CHECK(country.response->candidates.front().id.starts_with("pinyin:"));
   std::cout<<"PASS production lexicon: nihao -> 你好, zhongguo -> 中国\n";
  } else {
   std::cout<<"PASS fallback IPC routing with explicit test lexicon\n";
  }
  document.invalidate();CHECK(client.close().get().status);
 }
 std::cout<<"PASS real IPC and simulated document commit (not actual Windows TSF SetText)\n";
 std::cout<<"PASS actual AppContainer Wubi process: permission denial, discovery, production queries, selection, pages, revoke, restart\n";
 std::cout<<"Retained isolated test store: "<<root.string()<<'\n';return 0;
}catch(const std::exception& e){std::cerr<<e.what()<<'\n';return 1;}}

