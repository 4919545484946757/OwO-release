#pragma once
#include <Windows.h>
#include <msctf.h>
#include <wrl/client.h>
#include <mutex>
#include <optional>
#include <string>
#include <thread>
#include <utility>

namespace owo::tsf {
class TextService;
inline constexpr UINT_PTR kSelectionAssistantTimer = 0x4f574f41;
#ifdef OWO_TSF_TEST_ACCESS
std::pair<bool, std::wstring> test_selection_worker(const wchar_t* executable);
#endif
class SelectionAssistant final {
public:
    explicit SelectionAssistant(TextService& service) : service_(service) {}
    ~SelectionAssistant();
    bool active() const noexcept { return active_; }
    bool test(ITfContext*, WPARAM);
    bool key(ITfContext*, WPARAM);
    void choose(std::size_t);
    void poll();
    void cancel();
    std::wstring header() const;
private:
#ifdef OWO_TSF_TEST_ACCESS
    friend struct SelectionAssistantTestAccess;
#endif
    void show();
    void start(std::size_t);
    bool unchanged(bool write, const std::wstring& replacement = {});
    TextService& service_;
    bool active_{}, busy_{}, failed_{};
    std::size_t operation_{};
    Microsoft::WRL::ComPtr<ITfContext> context_;
    Microsoft::WRL::ComPtr<ITfRange> range_;
    HWND focus_{};
    std::wstring selection_, surrounding_, application_, error_;
    std::jthread worker_;
    std::mutex mutex_;
    struct Result { bool ok{}; std::wstring text; };
    std::optional<Result> result_;
};
}
