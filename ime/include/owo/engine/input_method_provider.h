#pragma once

#include <cstddef>
#include <cstdint>
#include <memory>
#include <mutex>
#include <stop_token>
#include <string>
#include <string_view>
#include <unordered_map>
#include <vector>

namespace owo::engine {

// Experimental host-side contract; not a stable binary or wire ABI.
inline constexpr std::uint32_t kInputMethodApiVersion = 1;

enum class InputMethodKind { phonetic, table, transliteration };
enum class InputMethodAction { compose, select, cancel };
enum class InputMethodStatus { success, cancelled, unavailable, error };

struct InputMethodDescriptor {
    std::uint32_t api_version{kInputMethodApiVersion};
    // Plugin-owned IDs: <plugin_id>/<local_scheme_id>.
    std::string id;
    std::string plugin_id;
    std::string name;
    std::string language_tag;
    InputMethodKind kind{InputMethodKind::table};
    // Unicode scalars accepted as encoding characters. No pinyin assumption.
    std::u32string alphabet;
    std::size_t max_input_bytes{1024};
    std::size_t max_candidates{128};
};

// Half-open UTF-8 byte ranges refer to the original, unmodified input.
struct InputMethodRange {
    std::size_t begin{};
    std::size_t end{};
};

struct InputMethodCandidate {
    std::string id;
    std::string text;
    std::string annotation;
    std::vector<InputMethodRange> consumed_ranges;
};

struct InputMethodRequest {
    std::uint64_t request_id{};
    std::string session_id;
    std::string scheme_id;
    InputMethodAction action{InputMethodAction::compose};
    std::string input;
    std::size_t caret_byte{};
    std::size_t page{};
    std::size_t page_size{7};
    std::string selected_candidate_id;
};

struct InputMethodResponse {
    std::uint64_t request_id{};
    std::string session_id;
    std::string scheme_id;
    InputMethodStatus status{InputMethodStatus::success};
    // Display text only, never a source for commands or consumption offsets.
    std::string preview;
    std::vector<InputMethodCandidate> candidates;
    std::size_t page{};
    bool has_more{};
    std::string diagnostic;
};

struct InputMethodValidation {
    bool ok{};
    std::string diagnostic;
};

[[nodiscard]] InputMethodValidation validate_input_method_descriptor(const InputMethodDescriptor&);
[[nodiscard]] InputMethodValidation validate_input_method_request(const InputMethodDescriptor&,
                                                                 const InputMethodRequest&);
[[nodiscard]] InputMethodValidation validate_input_method_response(const InputMethodDescriptor&,
                                                                  const InputMethodRequest&,
                                                                  const InputMethodResponse&);

class InputMethodProvider {
public:
    virtual ~InputMethodProvider() = default;
    // Called by a host worker, not a TSF key callback. Implementations must
    // support cancellation and serialize their own state if needed.
    [[nodiscard]] virtual InputMethodResponse handle(const InputMethodRequest&,
                                                     std::stop_token) = 0;
};

// Registry owns metadata and provider lifetimes; it does not confer permission
// or load third-party code. PluginHost authorization precedes registration.
class InputMethodRegistry {
public:
    [[nodiscard]] InputMethodValidation add(InputMethodDescriptor,
                                            std::shared_ptr<InputMethodProvider>);
    void remove_plugin(std::string_view plugin_id);
    [[nodiscard]] std::vector<InputMethodDescriptor> descriptors() const;
    [[nodiscard]] InputMethodResponse dispatch(const InputMethodRequest&,
                                               std::stop_token = {}) const;
private:
    struct Entry {
        InputMethodDescriptor descriptor;
        std::shared_ptr<InputMethodProvider> provider;
        std::uint64_t generation{};
    };
    mutable std::mutex mutex_;
    std::unordered_map<std::string, Entry> entries_;
    std::uint64_t next_generation_{1};
};

} // namespace owo::engine
