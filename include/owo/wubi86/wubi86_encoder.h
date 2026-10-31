#pragma once
#include "wubi86_code_table.h"
namespace owo::wubi86 {
class Wubi86Encoder {
public:
 // Own a snapshot, so caller mutations/lifetimes cannot invalidate encoding.
 explicit Wubi86Encoder(Wubi86CodeTable table) : table_(std::move(table)) {}
 [[nodiscard]] EncodeResult encode(std::string_view text, std::size_t max_variants = 4096) const;
private:
 Wubi86CodeTable table_;
};
}
