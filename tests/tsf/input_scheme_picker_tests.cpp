#include "owo/tsf/input_scheme_picker.h"
#include "owo/plugin/input_scheme_icon.h"

#include <iostream>
#include <string>

#define CHECK(value) do { if (!(value)) { std::cerr << #value << '\n'; return 1; } } while (false)

int main() {
    owo::tsf::InputSchemePicker picker;
    CHECK(!picker.tap(1, 0) && !picker.showing());
    CHECK(picker.tap(3, 1) && picker.selection() == 2);
    CHECK(!picker.tap(3, 1) && picker.selection() == 2);
    picker.release_space();
    CHECK(picker.tap(3, {}) && picker.selection() == 0);
    picker.close();
    CHECK(!picker.showing() && !picker.tap(3, 0));
    picker.release_space();
    CHECK(picker.tap(3, {}) && picker.selection() == 0);
    picker.reset();
    CHECK(picker.tap(3, 0) && picker.selection() == 1);

    std::string mask = "OWO_SCHEME_ICON_MASK_V1\r\n";
    for (int i = 0; i < 16; ++i) mask += "1000000000000001\r\n";
    const auto icon = owo::plugin::parse_input_scheme_icon(mask);
    CHECK(icon && icon->front() == 0x8001 && icon->back() == 0x8001);
    CHECK(!owo::plugin::parse_input_scheme_icon(mask + "extra"));
    CHECK(!owo::plugin::parse_input_scheme_icon(mask.substr(0, mask.size() - 4)));
    mask[24] = 'X';
    CHECK(!owo::plugin::parse_input_scheme_icon(mask));
    CHECK(!owo::plugin::read_input_scheme_icon(".", "../bad"));
}
