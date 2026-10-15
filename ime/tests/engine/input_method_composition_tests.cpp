#include "owo/engine/input_method_composition.h"
#include <iostream>

using namespace owo::engine;
namespace {
int failures{};
void check(bool value, const char* why) { if (!value) { ++failures; std::cerr << why << '\n'; } }
InputMethodResponse result(const InputMethodRequest& r) {
    InputMethodResponse v;
    v.request_id = r.request_id; v.session_id = r.session_id; v.scheme_id = r.scheme_id;
    v.page = r.page; v.preview = "display only"; return v;
}
}
int main() {
    InputMethodDescriptor d;
    d.plugin_id = "example.input"; d.id = "example.input/table"; d.name = "Test";
    d.language_tag = "zh-Hans"; d.alphabet = U"aAbeé'😀"; d.max_input_bytes = 16;
    InputMethodComposition c;
    check(c.begin(d, "session"), "valid begin");
    check(c.insert("Aé😀b") && c.input() == "Aé😀b" && c.caret_byte() == 8 && c.caret_utf16() == 5,
        "Unicode/case preserved");
    check(!c.insert("z") && !c.insert(std::string("\xc0\xaf", 2)) && c.input() == "Aé😀b",
        "invalid insertion atomic");
    check(!c.set_caret_utf16(3) && c.caret_byte() == 8, "surrogate interior rejected");
    check(!c.set_caret_utf16(99) && c.caret_byte() == 8, "out of range caret rejected");
    check(c.set_caret_utf16(4) && c.backspace() && c.input() == "Aéb" && c.caret_byte() == 3,
        "delete entire supplementary scalar");
    check(c.move_caret(-1) && c.erase_forward() && c.input() == "Ab", "forward delete entire UTF8 scalar");
    check(c.insert("é'") && c.input() == "Aé'b", "explicit separator and middle insertion");
    auto r = c.compose_request(1, 0, 7);
    check(r.has_value(), "issue composition");
    auto response = result(*r);
    response.candidates = {{"multi", "选词", {}, {{0, 1}, {3, 4}}}};
    check(c.apply(response).accepted && c.selectable(), "valid candidate snapshot");
    check(!c.select_request(2, "unknown") && c.selectable(), "unknown ID cannot consume");
    auto selection = c.select_request(2, "multi");
    check(selection && selection->page_size == 7 && !c.selectable() && c.input() == "Aé'b",
        "select does not consume before acknowledgement");
    auto wrong = result(*selection); ++wrong.request_id;
    check(!c.apply(wrong).accepted && c.input() == "Aé'b", "wrong acknowledgement ignored");
    auto applied = c.apply(result(*selection));
    check(applied.accepted && applied.commit && applied.commit->text == "选词" &&
        applied.commit->consumed_input == std::vector<std::string>{"A", "'"} &&
        c.input() == "Aé'b", "selection acknowledgement prepares original ranges without consuming");
    check(!c.insert("a") && !c.backspace() && !c.move_caret(1) && !c.compose_request(3, 0, 7),
        "prepared commit freezes raw snapshot until host acknowledgement");
    check(c.finish_commit(true) && c.input() == "éb" && c.caret_byte() == 2,
        "host commit consumes disjoint ranges and remaps caret");
    check(!c.finish_commit(true), "commit cannot consume twice");
    check(!c.apply(result(*selection)).accepted, "selection cannot commit twice");
    auto current = c.compose_request(3, 0, 7);
    auto stale = result(*current); stale.candidates = {{"one", "字", {}, {{0, 2}}}};
    check(c.insert("a") && !c.apply(stale).accepted && !c.selectable(), "edit invalidates in-flight result");
    current = c.compose_request(4, 0, 7); auto shown = result(*current);
    shown.candidates = {{"one", "字", {}, {{0, 2}}}};
    check(c.apply(shown).accepted, "updated response accepted");
    auto next = c.compose_request(5, 1, 7);
    check(next && c.display() && c.display()->request_id == 4 && !c.selectable(),
        "retain visual snapshot while new page loads but disable stale selection");
    auto bad = result(*next); bad.candidates = {{"bad", "字", {}, {{1, 2}}}};
    check(!c.apply(bad).accepted, "UTF8 interior range rejected");
    auto error = result(*next); error.status = InputMethodStatus::error;
    const auto input = c.input();
    check(c.apply(error).accepted && c.input() == input && !c.selectable(), "provider failure preserves raw input");
    current = c.compose_request(6, 0, 7); shown = result(*current);
    shown.candidates = {{"one", "字", {}, {{0, 2}}}};
    check(c.apply(shown).accepted, "candidate recovered");
    selection = c.select_request(7, "one");
    auto failed_selection = result(*selection); failed_selection.status = InputMethodStatus::unavailable;
    check(c.apply(failed_selection).accepted && c.input() == input, "failed selection does not consume");
    current = c.compose_request(8, 0, 7); shown = result(*current);
    shown.candidates = {{"one", "字", {}, {{0, 2}}}};
    check(c.apply(shown).accepted, "prepare host failure test");
    selection = c.select_request(9, "one");
    check(c.apply(result(*selection)).commit.has_value() && c.finish_commit(false) && c.input() == input,
        "host application failure preserves all encoding");
    auto final = c.compose_request(10, 0, 7);
    c.close();
    check(!c.apply(result(*final)).accepted && !c.insert("a"), "closed composition rejects late work");
    check(c.begin(d, "session") && !c.compose_request(10, 0, 7) && c.compose_request(11, 0, 7),
        "reopening cannot reuse request identity");
    auto invalid = d; invalid.alphabet = {};
    check(!c.begin(invalid, "other") && c.insert("a"), "invalid scheme switch preserves old state");
    check(!c.insert(std::string(16, 'a')) && c.input() == "a", "byte limit enforced without mutation");
    return failures ? 1 : 0;
}
