#pragma once

#include "owo/config/config_store.h"
#include "owo/agent/agent_protocol.h"
#include "owo/plugin/candidate_theme.h"
#include "owo/plugin/unicode_palette.h"
#include "owo/tsf/input_method_session.h"
#include "owo/tsf/key_routing.h"

#include <Windows.h>
#include <msctf.h>

#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdint>
#include <deque>
#include <memory>
#include <mutex>
#include <optional>
#include <string>
#include <string_view>
#include <thread>
#include <vector>

struct ID2D1Factory;
struct ID2D1DCRenderTarget;
struct ID2D1SolidColorBrush;
struct IDWriteFactory;
struct IDWriteTextFormat;

namespace owo::tsf {

class LanguageBarItem;
enum class NativeWriteResult;

inline constexpr CLSID kTextServiceClsid{
    0x6d31c9b1, 0x8978, 0x4f49, {0x89, 0xb4, 0x66, 0xeb, 0x1e, 0x74, 0x15, 0x91}};
inline constexpr GUID kLanguageProfileGuid{
    0x5d9f39c3, 0xbdb4, 0x453c, {0xa7, 0xba, 0xb9, 0xef, 0x82, 0x48, 0x76, 0x29}};
inline constexpr GUID kLanguageModePreservedKeyGuid{
    0x409a6b54, 0xd599, 0x4d7d, {0xa4, 0x36, 0xa1, 0x80, 0x60, 0xf5, 0xe1, 0x81}};
inline constexpr LANGID kSimplifiedChinese = 0x0804;

class TextService final : public ITfTextInputProcessorEx,
                          public ITfKeyEventSink,
                          public ITfThreadMgrEventSink,
                          public ITfThreadFocusSink,
                          public ITfCompartmentEventSink,
                          public ITfTextEditSink {
public:
    TextService() noexcept;

    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID iid, void** object) override;
    ULONG STDMETHODCALLTYPE AddRef() override;
    ULONG STDMETHODCALLTYPE Release() override;

    HRESULT STDMETHODCALLTYPE Activate(ITfThreadMgr* thread_manager, TfClientId client_id) override;
    HRESULT STDMETHODCALLTYPE ActivateEx(ITfThreadMgr* thread_manager,
                                         TfClientId client_id,
                                         DWORD flags) override;
    HRESULT STDMETHODCALLTYPE Deactivate() override;

    HRESULT STDMETHODCALLTYPE OnSetFocus(BOOL foreground) override;
    HRESULT STDMETHODCALLTYPE OnTestKeyDown(ITfContext* context,
                                            WPARAM key,
                                            LPARAM flags,
                                            BOOL* eaten) override;
    HRESULT STDMETHODCALLTYPE OnKeyDown(ITfContext* context,
                                        WPARAM key,
                                        LPARAM flags,
                                        BOOL* eaten) override;
    HRESULT STDMETHODCALLTYPE OnTestKeyUp(ITfContext* context,
                                          WPARAM key,
                                          LPARAM flags,
                                          BOOL* eaten) override;
    HRESULT STDMETHODCALLTYPE OnKeyUp(ITfContext* context,
                                      WPARAM key,
                                      LPARAM flags,
                                      BOOL* eaten) override;
    HRESULT STDMETHODCALLTYPE OnPreservedKey(ITfContext* context,
                                             REFGUID guid,
                                             BOOL* eaten) override;

    HRESULT STDMETHODCALLTYPE OnInitDocumentMgr(ITfDocumentMgr* document_manager) override;
    HRESULT STDMETHODCALLTYPE OnUninitDocumentMgr(ITfDocumentMgr* document_manager) override;
    HRESULT STDMETHODCALLTYPE OnSetFocus(ITfDocumentMgr* document_manager,
                                         ITfDocumentMgr* previous_document_manager) override;
    HRESULT STDMETHODCALLTYPE OnPushContext(ITfContext* context) override;
    HRESULT STDMETHODCALLTYPE OnPopContext(ITfContext* context) override;

    HRESULT STDMETHODCALLTYPE OnSetThreadFocus() override;
    HRESULT STDMETHODCALLTYPE OnKillThreadFocus() override;

    HRESULT STDMETHODCALLTYPE OnChange(REFGUID guid) override;
    HRESULT STDMETHODCALLTYPE OnEndEdit(ITfContext* context, TfEditCookie cookie,
                                         ITfEditRecord* record) override;

private:
    friend class LanguageBarItem;
    struct CandidateResult {
        std::uint64_t request_id{};
        std::uint64_t generation{};
        std::uint64_t page{};
        bool has_more{};
        bool expanded{};
        std::uint64_t page_size{7};
        bool preserve_paging{};
        bool request_failed{};
        bool refinement_pending{};
        std::wstring failure_detail;
        std::wstring segmented_input;
        std::vector<std::wstring> candidates;
        std::vector<std::uint64_t> candidate_consumed;
        std::vector<std::uint64_t> candidate_groups;
    };
    struct AgentResult {
        std::uint64_t request_id{};
        std::uint64_t generation{};
        agent::Response response;
        bool transport_failure{};
    };
    struct PendingRequest {
        std::uint8_t type{};
        std::uint64_t request_id{};
        std::uint64_t generation{};
        std::uint64_t page{};
        std::wstring input;
        bool expanded{};
        bool correction_enabled{true};
        std::wstring feedback_input;
        std::wstring language_context;
        agent::ApplicationContext application_context;
        std::chrono::steady_clock::time_point queued_at{std::chrono::steady_clock::now()};
        bool defer_expensive_candidates{};
    };
    enum class HitKind : std::uint8_t {
        pinyin_cursor,
        candidate,
        previous_page,
        next_page,
        toggle_expanded,
        unicode_toggle,
        unicode_category,
        unicode_item,
    };
    struct HitTarget {
        HitKind kind{HitKind::candidate};
        std::size_t candidate_index{};

        bool operator==(const HitTarget&) const = default;
    };
    struct HitRegion {
        RECT bounds{};
        HitTarget target;
    };

    virtual ~TextService();
    static LRESULT CALLBACK window_proc(HWND window, UINT message, WPARAM wparam, LPARAM lparam);
    static LRESULT CALLBACK keyboard_hook_proc(int code, WPARAM key, LPARAM flags);
    HRESULT initialize_windows();
    void destroy_windows() noexcept;
    HRESULT initialize_rendering();
    HRESULT ensure_device_resources(SIZE surface_size);
    void discard_device_resources() noexcept;
    void discard_rendering() noexcept;
    void apply_candidate_window_effects() noexcept;
    bool prepare_blurred_candidate_background() noexcept;
    [[nodiscard]] bool present_candidate_surface() noexcept;
    void maintain_candidate_window_order() noexcept;
    void update_candidate_shadow() noexcept;
    void hide_candidate_windows() noexcept;
    [[nodiscard]] bool render_candidate_window();
    [[nodiscard]] SIZE desired_candidate_window_size() const;
    void worker_loop(std::stop_token stop_token);
    void queue_candidate_request();
    void schedule_candidate_request(bool reset_retry);
    [[nodiscard]] bool is_agent_mode() const noexcept;
    void queue_agent_action(agent::Action action, std::string command_id = {},
                            std::uint32_t page = 0,
                            std::uint64_t task_revision = 0,
                            std::vector<agent::SlotUpdate> slot_updates = {});
    bool select_agent_command(std::size_t index);
    void apply_agent_task(const agent::TaskDraft& task);
    void cancel_agent_session();
    void handle_agent_result(AgentResult* result);
    [[nodiscard]] std::wstring agent_preview_text() const;
    void queue_commit_feedback(std::wstring candidate, std::wstring input,
                               std::wstring context);
    void queue_agent_context(std::wstring text);
    void rotate_agent_context();
    void append_language_context(std::wstring_view text);
    void refresh_shortcut_config(bool force = false);
    void refresh_candidate_theme(bool force = false);
    void toggle_unicode_palette();
    void change_unicode_palette_page(int direction);
    void scroll_unicode_palette_rows(int rows);
    void scroll_unicode_categories(float dip_delta);
    HRESULT commit_unicode_item_from_window(std::size_t item_index);
    void sync_preserved_language_key();
    void clear_preserved_language_key() noexcept;
    void initialize_language_compartment();
    void uninitialize_language_compartment() noexcept;
    [[nodiscard]] std::optional<bool> system_language_mode() const;
    HRESULT apply_language_mode(bool chinese, ITfContext* context);
    void initialize_language_bar();
    void uninitialize_language_bar() noexcept;
    void update_language_bar() noexcept;
    [[nodiscard]] bool shortcut_matches(const std::vector<std::string>& shortcuts,
                                        WPARAM key) const;
    void handle_candidate_result(CandidateResult* result);
    void update_candidate_window();
    void move_pinyin_cursor(int direction);
    HRESULT move_document_caret(ITfContext* context, WPARAM key, bool select);
    [[nodiscard]] bool has_composition() const noexcept;
    [[nodiscard]] std::wstring staged_text() const;
    [[nodiscard]] std::wstring composition_preview() const;
    [[nodiscard]] std::size_t composition_cursor() const noexcept;
    void set_composition_cursor(std::size_t cursor);
    void restore_staged_suffix(std::size_t first);
    HRESULT commit_pending_composition(ITfContext* context);
    void commit_pending_on_focus_loss();
    void move_candidate_page_from_shortcut(int direction);
    void change_candidate_page(int direction);
    void scroll_expanded_candidates(int rows);
    void append_alt_code_digit(unsigned digit);
    void finish_alt_code_preview() noexcept;
    HRESULT commit_alt_code_preview_from_window();
    void change_alt_code_preview_page(int direction);
    void invoke_hit_target(const HitTarget& target);
    void defer_candidate_selection(std::size_t index, ITfContext* context);
    void clear_deferred_candidate_selection() noexcept;
    void defer_punctuation(std::wstring punctuation, ITfContext* context);
    void clear_deferred_punctuation() noexcept;
    [[nodiscard]] std::optional<HitTarget> hit_test(POINT point) const;
    void update_candidate_anchor(ITfContext* context);
    void clear_composition();
    [[nodiscard]] bool should_eat_key(WPARAM key) const noexcept;
    // Routes a digit key against the current composition state. Candidate
    // selection keeps priority; a digit after a reading character that does not
    // select a candidate joins the composition so modeless mixed input stays in
    // the preedit.
    [[nodiscard]] owo::tsf::DigitKeyAction digit_key_action(WPARAM key) const noexcept;
    [[nodiscard]] std::optional<std::wstring> chinese_punctuation_for_key(
        WPARAM key, bool advance_quote_state = false);
    HRESULT commit_candidate(ITfContext* context, std::size_t index,
                             std::wstring_view suffix = {});
    HRESULT stage_agent_candidate(ITfContext* context, std::size_t index);
    HRESULT commit_text(ITfContext* context, std::wstring text);
    HRESULT commit_raw_input(ITfContext* context);
    HRESULT commit_candidate_from_window(std::size_t index);
    HRESULT toggle_language_mode(ITfContext* context);
    HRESULT toggle_language_mode_from_window();

    LONG references_{1};
    bool plugin_scheme_selected() const noexcept;
    bool plugin_context_allowed(ITfContext*);
    void refresh_agent_context_safety(ITfContext*);
    // Returns true only when the effect was written (document or clipboard).
    NativeWriteResult apply_agent_effect(const agent::CommandEffect& effect);
    bool capture_agent_origin(ITfContext* context);
    void reset_agent_origin() noexcept;
    bool plugin_context_is_current(ITfContext*) const;
    bool plugin_should_eat_key(ITfContext*, WPARAM, LPARAM);
    HRESULT handle_plugin_key(ITfContext*, WPARAM, LPARAM, BOOL*);
    void poll_plugin_input();
    void finish_plugin_input_on_ui();
    void queue_plugin_compose();
    void select_plugin_candidate(std::size_t, bool defer = false);
    void change_plugin_page(int);
    void sync_plugin_display();
    void reset_plugin_input();
    void commit_plugin_text(std::wstring, bool selected, bool switch_language = false);
    // Explicitly selected, independently authorized plugin scheme path.
    InputMethodDocumentSession plugin_input_document_;
    std::unique_ptr<ipc::InputMethodAsyncClient> plugin_input_client_;
    ITfContext* plugin_input_context_{nullptr};
    ITfRange* plugin_input_selection_{nullptr};
    ITfContext* plugin_replay_context_{nullptr};
    ITfRange* plugin_replay_selection_{nullptr};
    std::uint64_t plugin_replay_generation_{};
    bool plugin_input_needs_compose_{};
    bool plugin_switch_after_commit_{};
    ULONGLONG plugin_commit_deadline_{};
    std::optional<engine::InputMethodDescriptor> plugin_input_descriptor_;
    std::future<ipc::InputMethodClientResult> plugin_input_future_;
    std::string plugin_input_session_;
    std::uint64_t plugin_input_request_id_{};
    struct PluginPendingSelection { std::size_t index; std::wstring input; std::uint64_t generation, page; };
    std::optional<PluginPendingSelection> plugin_pending_selection_;
    bool plugin_input_opening_{}, plugin_input_committing_{};
    struct PluginDeferredKey { WPARAM key; LPARAM flags; BYTE state[256]; };
    std::deque<PluginDeferredKey> plugin_deferred_keys_;
    ITfThreadMgr* thread_manager_{nullptr};
    TfClientId client_id_{TF_CLIENTID_NULL};
    DWORD thread_manager_event_sink_cookie_{TF_INVALID_COOKIE};
    DWORD thread_focus_sink_cookie_{TF_INVALID_COOKIE};
    DWORD language_compartment_sink_cookie_{TF_INVALID_COOKIE};
    HWND message_window_{nullptr};
    HWND candidate_shadow_window_{nullptr};
    HWND candidate_window_{nullptr};
    bool candidate_resize_in_progress_{};
    POINT candidate_present_position_{};
    SIZE candidate_present_size_{};
    SIZE candidate_content_size_{};
    bool candidate_present_geometry_valid_{};
    HHOOK keyboard_hook_{nullptr};
    bool language_space_key_suppressed_{false};
    std::uint16_t alt_code_numpad_down_mask_{0};
    LanguageBarItem* language_bar_item_{nullptr};
    ID2D1Factory* d2d_factory_{nullptr};
    IDWriteFactory* dwrite_factory_{nullptr};
    IDWriteTextFormat* input_text_format_{nullptr};
    IDWriteTextFormat* candidate_text_format_{nullptr};
    IDWriteTextFormat* label_text_format_{nullptr};
    ID2D1DCRenderTarget* render_target_{nullptr};
    HDC layered_surface_dc_{nullptr};
    HBITMAP layered_surface_bitmap_{nullptr};
    HGDIOBJ layered_surface_previous_bitmap_{nullptr};
    void* layered_surface_pixels_{nullptr};
    SIZE layered_surface_size_{};
    std::vector<std::uint32_t> blurred_candidate_background_;
    SIZE candidate_shadow_surface_size_{};
    owo::plugin::CandidateTheme candidate_shadow_theme_{};
    bool candidate_shadow_surface_valid_{false};
    ID2D1SolidColorBrush* background_brush_{nullptr};
    ID2D1SolidColorBrush* border_brush_{nullptr};
    ID2D1SolidColorBrush* divider_brush_{nullptr};
    ID2D1SolidColorBrush* text_brush_{nullptr};
    ID2D1SolidColorBrush* secondary_text_brush_{nullptr};
    ID2D1SolidColorBrush* accent_brush_{nullptr};
    ID2D1SolidColorBrush* highlight_brush_{nullptr};
    ID2D1SolidColorBrush* strict_accent_brush_{nullptr};
    struct StagedCandidate {
        std::wstring text;
        std::wstring source_input;
    };
    std::wstring input_buffer_;
    std::size_t input_cursor_{0};
    std::vector<StagedCandidate> staged_candidates_;
    std::size_t staged_cursor_{0};
    std::wstring learning_chain_text_;
    std::wstring learning_chain_input_;
    std::wstring learning_chain_context_;
    std::wstring recent_language_context_;
    std::wstring recent_agent_context_;
    std::wstring recent_language_input_;
    std::wstring segmented_input_;
    std::vector<std::wstring> candidates_;
    std::vector<std::uint64_t> candidate_consumed_;
    std::vector<std::uint64_t> candidate_groups_;
    std::string agent_session_id_;
    std::string agent_context_id_;
    bool agent_context_sensitive_{true};
    bool agent_effects_negotiated_{false};
    ITfContext* agent_origin_context_{nullptr};
    ITfRange* agent_origin_range_{nullptr};
    ITfSource* agent_origin_source_{nullptr};
    DWORD agent_edit_sink_cookie_{TF_INVALID_COOKIE};
    bool agent_origin_valid_{false};
    bool agent_origin_safety_blocked_{false};
    std::string agent_origin_sha256_;
    std::optional<agent::SelectionSummary> agent_origin_selection_;
    agent::Status agent_status_{agent::Status::agent_mode};
    std::wstring agent_message_;
    std::wstring agent_executing_command_;
    std::wstring agent_last_submitted_pinyin_;
    std::string agent_last_request_id_;
    std::uint64_t agent_state_revision_{};
    agent::TaskDraft agent_task_;
    std::vector<agent::Command> agent_commands_;
    bool agent_request_submitted_{false};
    bool agent_request_pending_{false};
    bool agent_response_visible_{false};
    std::wstring candidate_failure_detail_;
    std::vector<HitRegion> hit_regions_;
    std::optional<HitTarget> hovered_target_;
    std::optional<HitTarget> pressed_target_;
    std::optional<std::wstring> deferred_candidate_text_;
    ITfContext* deferred_candidate_context_{nullptr};
    std::optional<std::wstring> deferred_punctuation_;
    ITfContext* deferred_punctuation_context_{nullptr};
    std::uint64_t context_generation_{0};
    std::uint64_t next_request_id_{1};
    std::uint64_t active_candidate_request_id_{0};
    std::atomic<std::uint64_t> latest_candidate_request_id_{0};
    HANDLE candidate_cancellation_event_{nullptr};
    std::vector<HANDLE> retired_cancellation_events_;
    std::uint64_t candidate_page_{0};
    std::uint64_t candidate_page_size_{7};
    bool has_more_candidates_{false};
    bool candidate_request_pending_{false};
    int deferred_page_direction_{0};
    bool candidate_request_failed_{false};
    std::uint8_t candidate_retry_count_{0};
    bool candidates_expanded_{false};
    std::size_t expanded_scroll_row_{0};
    config::ConfigStore shortcut_config_store_;
    config::AppConfig shortcut_config_;
    ULONGLONG next_shortcut_config_refresh_{0};
    bool shortcut_config_initialized_{false};
    plugin::CandidateTheme candidate_theme_{};
    std::string candidate_theme_source_;
    plugin::UnicodePalette unicode_palette_{};
    std::string unicode_palette_source_;
    bool unicode_palette_visible_{false};
    std::size_t unicode_category_index_{0};
    std::size_t unicode_scroll_row_{0};
    float unicode_category_scroll_offset_dip_{0.0F};
    POINT unicode_hover_point_{};
    ULONGLONG next_candidate_theme_refresh_{0};
    std::vector<std::string> preserved_language_shortcuts_;
    std::vector<TF_PRESERVEDKEY> preserved_language_keys_;
    bool correction_enabled_{true};
    bool chinese_mode_{true};
    bool chinese_punctuation_enabled_{true};
    bool single_quote_open_{true};
    bool double_quote_open_{true};
    bool foreground_focus_{true};
    struct AltCodePreviewItem {
        std::wstring code;
        std::wstring result;
        bool strict{};
    };
    bool alt_code_preview_active_{false};
    std::wstring alt_code_digits_;
    std::vector<AltCodePreviewItem> alt_code_preview_items_;
    std::size_t alt_code_preview_page_{0};
    POINT candidate_anchor_{};
    bool candidate_anchor_valid_{false};
    std::mutex request_mutex_;
    std::condition_variable_any request_ready_;
    std::optional<PendingRequest> pending_request_;
    std::deque<PendingRequest> feedback_requests_;
    std::jthread worker_;
};

HRESULT create_text_service(REFIID iid, void** object);
void increment_server_lock() noexcept;
void decrement_server_lock() noexcept;
[[nodiscard]] long server_lock_count() noexcept;

}  // namespace owo::tsf
