#include "owo/protocol/envelope.h"

#include <iostream>
#include <string>

namespace {

int failures = 0;

void expect(const bool condition, const char* message) {
    if (!condition) {
        std::cerr << "FAILED: " << message << '\n';
        ++failures;
    }
}

}  // namespace

int main() {
    using namespace owo::protocol;

    Envelope valid{};
    valid.request_id = 42;
    valid.context_generation = 7;
    valid.payload_json = R"({"type":"candidate_request"})";
    expect(static_cast<bool>(validate(valid)), "valid envelope must pass");

    Envelope wrong_version = valid;
    wrong_version.protocol_version = kProtocolVersion + 1;
    expect(validate(wrong_version).error == ErrorCode::unsupported_protocol,
           "unknown protocol version must fail explicitly");

    Envelope empty = valid;
    empty.payload_json.clear();
    expect(validate(empty).error == ErrorCode::invalid_payload,
           "empty payload must fail explicitly");

    Envelope oversized = valid;
    oversized.payload_json.assign(kMaximumPayloadBytes + 1U, 'x');
    expect(validate(oversized).error == ErrorCode::payload_too_large,
           "oversized payload must fail explicitly");

    const std::string payload = "abc";
    const auto framed = frame(payload);
    expect(framed.size() == 7, "frame must contain prefix and payload");
    expect(static_cast<unsigned char>(framed[0]) == 3U,
           "frame prefix must use little-endian payload size");
    expect(framed.substr(4) == payload, "frame payload must be preserved");

    return failures == 0 ? 0 : 1;
}
