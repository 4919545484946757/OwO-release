#pragma once
#include <filesystem>
#include <map>
#include <set>
#include <string>
#include <vector>
namespace owo::wubi86 {
// Per-plugin data path only; never writes the packaged dictionary.
class Learning {
public:
 explicit Learning(std::filesystem::path path={}):path_(std::move(path)){try{read();}catch(...){counts_.clear();tokens_.clear();order_.clear();}}
 std::uint32_t frequency(const std::string& code,const std::string& text) const;
 bool contains(const std::string& token) const {return tokens_.contains(token);}
 bool matches(const std::string& token,const std::string& binding) const {auto it=tokens_.find(token);return it!=tokens_.end()&&it->second==binding;}
 bool commit(const std::string& token,const std::string& code,const std::string& text,const std::string& binding);
private:
 bool read();bool write() const;
 std::filesystem::path path_;
 std::map<std::pair<std::string,std::string>,std::uint32_t> counts_;
 std::map<std::string,std::string> tokens_;std::vector<std::string> order_;
};
}
