#include "learning.h"
#include "owo/wubi86/wubi86_types.h"
#include <Windows.h>
#include <bcrypt.h>
#include <fstream>
#include <sstream>
#include <charconv>
namespace owo::wubi86 {
namespace {
bool token_valid(std::string_view s){if(s.size()!=32)return false;for(char c:s)if(!((c>='0'&&c<='9')||(c>='a'&&c<='f')))return false;return true;}
std::vector<std::string> split(std::string_view s){std::vector<std::string> v;std::size_t begin=0;for(;;){auto at=s.find('\t',begin);v.emplace_back(s.substr(begin,at==s.npos?at:at-begin));if(at==s.npos)return v;begin=at+1;}}
}
bool Learning::read(){
 if(path_.empty())return true;
 auto clear=[&]{counts_.clear();tokens_.clear();order_.clear();};
 auto quarantine=[&] {auto target=path_;target+=L".corrupt."+std::to_wstring(GetCurrentProcessId())+L"."+std::to_wstring(GetTickCount64());std::error_code ec;std::filesystem::rename(path_,target,ec);if(ec)return false;clear();return true;};
 std::error_code size_error;auto size=std::filesystem::file_size(path_,size_error);
 if(size_error){if(size_error==std::errc::no_such_file_or_directory){clear();return true;}return false;}
 if(size>2*1024*1024)return quarantine();
 std::ifstream in(path_,std::ios::binary);if(!in)return false;
 decltype(counts_) counts;decltype(tokens_) tokens;decltype(order_) order;
 bool valid=true;std::string line;std::size_t bytes=0;
 if(!std::getline(in,line)||line!="OWO-WUBI-LEARN\t1")valid=false;
 while(valid&&std::getline(in,line)){
  bytes+=line.size();if(bytes>2*1024*1024){valid=false;break;}auto f=split(line);
  if(f.size()==4&&f[0]=="entry"){
   auto scalar=utf8_scalar_count(f[2]);std::uint32_t n{};auto parsed=std::from_chars(f[3].data(),f[3].data()+f[3].size(),n);
   if(!valid_code(f[1])||!scalar||!*scalar||f[3].empty()||parsed.ec!=std::errc{}||parsed.ptr!=f[3].data()+f[3].size()||n==0||n>1000000||counts.size()>=4096||!counts.emplace(std::make_pair(f[1],f[2]),n).second){valid=false;break;}
  }else if(f.size()==3&&f[0]=="commit"&&token_valid(f[1])&&f[2].size()<=4096&&tokens.size()<4096&&tokens.emplace(f[1],f[2]).second)order.push_back(f[1]);else {valid=false;break;}
 }
 const bool io_error=in.bad();in.close();if(io_error)return false;
 if(!valid)return quarantine();
 counts_=std::move(counts);tokens_=std::move(tokens);order_=std::move(order);return true;
}

std::uint32_t Learning::frequency(const std::string& code,const std::string& text) const {if(counts_.empty())return 0;auto it=counts_.find({code,text});return it==counts_.end()?0:it->second;}
bool Learning::write() const {
 if(path_.empty())return true;
 std::ostringstream out;out<<"OWO-WUBI-LEARN\t1\n";for(const auto& [key,n]:counts_)out<<"entry\t"<<key.first<<'\t'<<key.second<<'\t'<<n<<'\n';for(const auto& token:order_)out<<"commit\t"<<token<<'\t'<<tokens_.at(token)<<'\n';auto data=out.str();if(data.size()>2*1024*1024)return false;
 BYTE random[16];if(BCryptGenRandom(nullptr,random,sizeof random,BCRYPT_USE_SYSTEM_PREFERRED_RNG)!=0)return false;
 constexpr wchar_t hex[]=L"0123456789abcdef";auto temp=path_;std::wstring suffix=L".pending.";for(auto c:random){suffix+=hex[c>>4];suffix+=hex[c&15];}temp+=suffix;
 HANDLE file=CreateFileW(temp.c_str(),GENERIC_WRITE,0,nullptr,CREATE_NEW,FILE_ATTRIBUTE_NORMAL,nullptr);if(file==INVALID_HANDLE_VALUE)return false;
 DWORD written{};bool ok=WriteFile(file,data.data(),static_cast<DWORD>(data.size()),&written,nullptr)&&written==data.size()&&FlushFileBuffers(file);CloseHandle(file);
 if(ok)ok=MoveFileExW(temp.c_str(),path_.c_str(),MOVEFILE_REPLACE_EXISTING|MOVEFILE_WRITE_THROUGH)!=0;
 if(!ok)DeleteFileW(temp.c_str()); // Only the unique file created above is removed.
 return ok;
}
bool Learning::commit(const std::string& token,const std::string& code,const std::string& text,const std::string& binding){
 const auto scalar=utf8_scalar_count(text),binding_scalar=utf8_scalar_count(binding);
 if(!token_valid(token)||!valid_code(code)||!scalar||!*scalar||!binding_scalar||!*binding_scalar||binding.size()>4096)return false;
 // Serialize writers from independent plugin processes with an exclusive sidecar handle.
 HANDLE lock=INVALID_HANDLE_VALUE;
 struct ReleaseLock {HANDLE& handle;~ReleaseLock(){if(handle!=INVALID_HANDLE_VALUE)CloseHandle(handle);}} release_lock{lock};
 if(!path_.empty()){auto name=path_;name+=L".lock";const auto deadline=GetTickCount64()+200;
  do {lock=CreateFileW(name.c_str(),GENERIC_READ|GENERIC_WRITE,0,nullptr,OPEN_ALWAYS,FILE_ATTRIBUTE_NORMAL,nullptr);if(lock!=INVALID_HANDLE_VALUE)break;if(GetLastError()!=ERROR_SHARING_VIOLATION||GetTickCount64()>=deadline)return false;Sleep(5);}while(true);if(!read())return false;}
 if(tokens_.contains(token)){const bool same=matches(token,binding);return same;}
 auto prior_counts=counts_;auto prior_tokens=tokens_;auto prior_order=order_;
 if(counts_.size()>=4096&&!counts_.contains({code,text})){return false;}
 auto& count=counts_[{code,text}];if(count<1000000)++count;
 if(order_.size()==4096){tokens_.erase(order_.front());order_.erase(order_.begin());}tokens_.emplace(token,binding);order_.push_back(token);
 bool ok=write();if(!ok){counts_=std::move(prior_counts);tokens_=std::move(prior_tokens);order_=std::move(prior_order);}return ok;
}
}
