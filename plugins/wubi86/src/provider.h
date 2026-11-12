#pragma once
#include "owo/engine/input_method_provider.h"
#include "owo/wubi86/wubi86_dictionary.h"
#include <mutex>
#include "learning.h"
#include <optional>
namespace owo::wubi86 {
engine::InputMethodDescriptor descriptor();
class Provider final : public engine::InputMethodProvider {
public:
 explicit Provider(const std::filesystem::path& dictionary,const std::filesystem::path& learning={},
                   const std::filesystem::path& scheme_data={});
 engine::InputMethodResponse handle(const engine::InputMethodRequest&,std::stop_token) override;
private:
 Wubi86Dictionary dictionary_;
 std::vector<Wubi86Dictionary> custom_dictionaries_;
 bool show_prefix_candidates_{true};
 Learning learning_;
 struct Commit {std::string token,session,input,code,text;std::size_t page;};
 std::optional<Commit> prepared_;
 std::mutex mutex_;
 std::string session_,input_;
 std::size_t page_{};
 std::uint64_t sequence_{};
 std::vector<engine::InputMethodCandidate> issued_;
};
}
