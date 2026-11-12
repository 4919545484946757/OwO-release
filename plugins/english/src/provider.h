#pragma once

#include "owo/engine/input_method_provider.h"

#include <filesystem>

namespace owo::english {

engine::InputMethodDescriptor descriptor();

class Provider final : public engine::InputMethodProvider {
public:
    Provider(const std::filesystem::path& wordfreq, std::filesystem::path data);
    ~Provider() override;
    engine::InputMethodResponse handle(const engine::InputMethodRequest&, std::stop_token) override;

private:
    struct State;
    std::unique_ptr<State> state_;
};

} // namespace owo::english
