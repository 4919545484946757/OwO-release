#include "owo/model/model_assets.h"

#ifdef _WIN32
#ifndef NOMINMAX
#define NOMINMAX
#endif
#include <Windows.h>
#include <bcrypt.h>
#endif

#include <algorithm>
#include <cctype>
#include <charconv>
#include <fstream>
#include <limits>
#include <map>
#include <unordered_set>

namespace owo::model {
namespace {

bool valid_identifier(const std::string_view value) {
    if (value.empty() || value.size() > 128) return false;
    return std::all_of(value.begin(), value.end(), [](const unsigned char byte) {
        return std::isalnum(byte) != 0 || byte == '.' || byte == '-' || byte == '_';
    });
}

bool valid_sha256(const std::string_view value) {
    return value.size() == 64 && std::all_of(value.begin(), value.end(), [](const unsigned char byte) {
        return std::isdigit(byte) != 0 || (byte >= 'a' && byte <= 'f');
    });
}

bool safe_asset_filename(const std::string_view value) {
    if (value.empty() || value.size() > 128) return false;
    const std::filesystem::path path(value);
    return !path.is_absolute() && !path.has_parent_path() && path.filename() == path;
}

bool parse_size(const std::string_view text, std::size_t& output) {
    std::uint64_t parsed{};
    const auto result = std::from_chars(text.data(), text.data() + text.size(), parsed);
    if (result.ec != std::errc{} || result.ptr != text.data() + text.size() ||
        parsed > std::numeric_limits<std::size_t>::max()) return false;
    output = static_cast<std::size_t>(parsed);
    return true;
}

bool read_varint(std::istream& input, std::uint64_t& value) {
    value = 0;
    for (unsigned int shift = 0; shift < 64; shift += 7) {
        const int next = input.get();
        if (next == std::char_traits<char>::eof()) return false;
        const auto byte = static_cast<unsigned char>(next);
        if (shift == 63 && (byte & 0x7eU) != 0) return false;
        value |= static_cast<std::uint64_t>(byte & 0x7fU) << shift;
        if ((byte & 0x80U) == 0) return true;
    }
    return false;
}

bool skip_bytes(std::istream& input, const std::uint64_t count) {
    if (count > static_cast<std::uint64_t>(std::numeric_limits<std::streamoff>::max())) return false;
    input.seekg(static_cast<std::streamoff>(count), std::ios::cur);
    return static_cast<bool>(input);
}

bool parse_default_opset_entry(const std::string_view bytes, std::uint64_t& version,
                               bool& default_domain) {
    std::size_t offset = 0;
    default_domain = true;
    bool has_version = false;
    auto read_memory_varint = [&](std::uint64_t& value) {
        value = 0;
        for (unsigned int shift = 0; shift < 64 && offset < bytes.size(); shift += 7) {
            const auto byte = static_cast<unsigned char>(bytes[offset++]);
            if (shift == 63 && (byte & 0x7eU) != 0) return false;
            value |= static_cast<std::uint64_t>(byte & 0x7fU) << shift;
            if ((byte & 0x80U) == 0) return true;
        }
        return false;
    };
    while (offset < bytes.size()) {
        std::uint64_t tag{};
        if (!read_memory_varint(tag)) return false;
        const auto field = tag >> 3U;
        const auto wire = tag & 7U;
        if (field == 1 && wire == 2) {
            std::uint64_t length{};
            if (!read_memory_varint(length) || length > bytes.size() - offset) return false;
            default_domain = length == 0;
            offset += static_cast<std::size_t>(length);
        } else if (field == 2 && wire == 0) {
            if (!read_memory_varint(version)) return false;
            has_version = true;
        } else if (wire == 0) {
            std::uint64_t ignored{};
            if (!read_memory_varint(ignored)) return false;
        } else if (wire == 1) {
            if (bytes.size() - offset < 8) return false;
            offset += 8;
        } else if (wire == 2) {
            std::uint64_t length{};
            if (!read_memory_varint(length) || length > bytes.size() - offset) return false;
            offset += static_cast<std::size_t>(length);
        } else if (wire == 5) {
            if (bytes.size() - offset < 4) return false;
            offset += 4;
        } else {
            return false;
        }
    }
    return has_version;
}

std::string sha256_file(const std::filesystem::path& path, const std::uintmax_t maximum_size) {
#ifdef _WIN32
    std::error_code error;
    const auto size = std::filesystem::file_size(path, error);
    if (error || size > maximum_size) return {};
    std::ifstream input(path, std::ios::binary);
    if (!input) return {};
    BCRYPT_ALG_HANDLE algorithm = nullptr;
    BCRYPT_HASH_HANDLE hash = nullptr;
    DWORD hash_size = 0;
    DWORD result_size = 0;
    if (BCryptOpenAlgorithmProvider(&algorithm, BCRYPT_SHA256_ALGORITHM, nullptr, 0) < 0 ||
        BCryptGetProperty(algorithm, BCRYPT_HASH_LENGTH,
                          reinterpret_cast<PUCHAR>(&hash_size), sizeof(hash_size),
                          &result_size, 0) < 0 ||
        BCryptCreateHash(algorithm, &hash, nullptr, 0, nullptr, 0, 0) < 0) {
        if (hash != nullptr) BCryptDestroyHash(hash);
        if (algorithm != nullptr) BCryptCloseAlgorithmProvider(algorithm, 0);
        return {};
    }
    std::vector<unsigned char> buffer(64U * 1024U);
    bool valid = true;
    while (input) {
        input.read(reinterpret_cast<char*>(buffer.data()), static_cast<std::streamsize>(buffer.size()));
        const auto count = input.gcount();
        if (count > 0 && BCryptHashData(hash, buffer.data(), static_cast<ULONG>(count), 0) < 0) {
            valid = false;
            break;
        }
    }
    if (!input.eof()) valid = false;
    std::vector<unsigned char> digest(hash_size);
    if (!valid || BCryptFinishHash(hash, digest.data(), hash_size, 0) < 0) valid = false;
    BCryptDestroyHash(hash);
    BCryptCloseAlgorithmProvider(algorithm, 0);
    if (!valid) return {};
    constexpr char hex[] = "0123456789abcdef";
    std::string output;
    output.reserve(digest.size() * 2);
    for (const auto byte : digest) {
        output.push_back(hex[byte >> 4U]);
        output.push_back(hex[byte & 0x0fU]);
    }
    return output;
#else
    static_cast<void>(path);
    static_cast<void>(maximum_size);
    return {};
#endif
}

bool decode_scalar(const std::string_view input, std::size_t& offset, std::string& scalar) {
    const auto first = static_cast<unsigned char>(input[offset]);
    std::size_t count = 0;
    std::uint32_t value = 0;
    if (first <= 0x7f) {
        count = 1;
        value = first;
    } else if (first >= 0xc2 && first <= 0xdf) {
        count = 2;
        value = first & 0x1fU;
    } else if (first >= 0xe0 && first <= 0xef) {
        count = 3;
        value = first & 0x0fU;
    } else if (first >= 0xf0 && first <= 0xf4) {
        count = 4;
        value = first & 0x07U;
    } else {
        return false;
    }
    if (offset + count > input.size()) return false;
    for (std::size_t index = 1; index < count; ++index) {
        const auto next = static_cast<unsigned char>(input[offset + index]);
        if ((next & 0xc0U) != 0x80U) return false;
        value = (value << 6U) | (next & 0x3fU);
    }
    if ((count == 3 && value < 0x800U) || (count == 4 && value < 0x10000U) ||
        (value >= 0xd800U && value <= 0xdfffU) || value > 0x10ffffU) return false;
    scalar.assign(input.substr(offset, count));
    offset += count;
    return true;
}

bool ascii_punctuation(const unsigned char byte) {
    return (byte >= 0x21 && byte <= 0x2f) || (byte >= 0x3a && byte <= 0x40) ||
           (byte >= 0x5b && byte <= 0x60) || (byte >= 0x7b && byte <= 0x7e);
}

}  // namespace

ValidationResult validate_manifest(const ModelManifest& manifest) {
    if (manifest.manifest_version != kModelManifestVersion)
        return {false, "unsupported manifest version"};
    if (!valid_identifier(manifest.model_id)) return {false, "invalid model id"};
    if (manifest.architecture != "bert") return {false, "unsupported architecture"};
    if (manifest.task != "candidate-ranking") return {false, "model task is not candidate-ranking"};
    if (manifest.format != "onnx") return {false, "unsupported model format"};
    if (manifest.source_revision.size() != 40 ||
        !std::all_of(manifest.source_revision.begin(), manifest.source_revision.end(),
                     [](const unsigned char byte) {
                         return std::isdigit(byte) != 0 || (byte >= 'a' && byte <= 'f');
                     })) return {false, "source revision must be a full lowercase commit sha"};
    if (!valid_sha256(manifest.model_sha256)) return {false, "invalid model sha256"};
    if (!valid_sha256(manifest.vocabulary_sha256)) return {false, "invalid vocabulary sha256"};
    if (manifest.license.empty() || manifest.license == "unknown")
        return {false, "model license is not explicit"};
    if (!safe_asset_filename(manifest.model_file)) return {false, "invalid model filename"};
    if (!safe_asset_filename(manifest.vocabulary_file))
        return {false, "invalid vocabulary filename"};
    if (manifest.onnx_opset < 13 || manifest.onnx_opset > 20)
        return {false, "ONNX opset is outside [13, 20]"};
    const std::vector<std::string_view> tensor_names{
        manifest.input_ids_name, manifest.attention_mask_name,
        manifest.token_type_ids_name, manifest.output_name};
    if (!std::all_of(tensor_names.begin(), tensor_names.end(), valid_identifier) ||
        std::unordered_set<std::string_view>(tensor_names.begin(), tensor_names.end()).size() !=
            tensor_names.size())
        return {false, "tensor names are invalid or duplicated"};
    if (manifest.input_element_type != "int64")
        return {false, "BERT input element type must be int64"};
    if (manifest.output_element_type != "float32")
        return {false, "ranking output element type must be float32"};
    if (manifest.output_columns != 1)
        return {false, "ranking output shape must be [batch, 1]"};
    if (manifest.maximum_sequence_length < 3 || manifest.maximum_sequence_length > 512)
        return {false, "maximum sequence length is outside [3, 512]"};
    if (manifest.maximum_candidates == 0 || manifest.maximum_candidates > 256)
        return {false, "maximum candidates is outside [1, 256]"};
    return {true, {}};
}

ValidationResult validate_onnx_metadata(const ModelManifest& manifest,
                                        const OnnxModelMetadata& metadata) {
    const auto manifest_validation = validate_manifest(manifest);
    if (!manifest_validation.ok) return manifest_validation;
    if (metadata.opset != manifest.onnx_opset) return {false, "ONNX opset does not match manifest"};
    if (metadata.inputs.size() != 3 || metadata.outputs.size() != 1)
        return {false, "ONNX graph must expose exactly three inputs and one output"};
    const std::vector<std::string_view> expected_inputs{
        manifest.input_ids_name, manifest.attention_mask_name, manifest.token_type_ids_name};
    for (const auto expected_name : expected_inputs) {
        const auto found = std::find_if(metadata.inputs.begin(), metadata.inputs.end(),
                                        [&](const auto& tensor) { return tensor.name == expected_name; });
        if (found == metadata.inputs.end()) return {false, "ONNX input name does not match manifest"};
        if (found->element_type != manifest.input_element_type)
            return {false, "ONNX input type does not match manifest"};
        if (found->dimensions != std::vector<std::int64_t>{
                                     -1, static_cast<std::int64_t>(manifest.maximum_sequence_length)})
            return {false, "ONNX input shape must be [dynamic_batch, fixed_sequence]"};
    }
    const auto& output = metadata.outputs.front();
    if (output.name != manifest.output_name || output.element_type != manifest.output_element_type)
        return {false, "ONNX output name or type does not match manifest"};
    if (output.dimensions != std::vector<std::int64_t>{
                                 -1, static_cast<std::int64_t>(manifest.output_columns)})
        return {false, "ONNX output shape must be [dynamic_batch, output_columns]"};
    return {true, {}};
}

OnnxOpsetResult read_onnx_default_opset(const std::filesystem::path& model_path) {
    std::ifstream input(model_path, std::ios::binary);
    if (!input) return {false, 0, "cannot open ONNX model"};
    while (input.peek() != std::char_traits<char>::eof()) {
        std::uint64_t tag{};
        if (!read_varint(input, tag)) return {false, 0, "invalid ONNX protobuf tag"};
        const auto field = tag >> 3U;
        const auto wire = tag & 7U;
        if (field == 8 && wire == 2) {
            std::uint64_t length{};
            if (!read_varint(input, length) || length > 1024U * 1024U)
                return {false, 0, "invalid ONNX opset entry"};
            std::string entry(static_cast<std::size_t>(length), '\0');
            if (!input.read(entry.data(), static_cast<std::streamsize>(entry.size())))
                return {false, 0, "truncated ONNX opset entry"};
            std::uint64_t version{};
            bool default_domain{};
            if (!parse_default_opset_entry(entry, version, default_domain))
                return {false, 0, "invalid ONNX opset entry"};
            if (default_domain) {
                if (version > std::numeric_limits<std::uint32_t>::max())
                    return {false, 0, "ONNX opset version is too large"};
                return {true, static_cast<std::uint32_t>(version), {}};
            }
        } else if (wire == 0) {
            std::uint64_t ignored{};
            if (!read_varint(input, ignored)) return {false, 0, "invalid ONNX varint"};
        } else if (wire == 1) {
            if (!skip_bytes(input, 8)) return {false, 0, "truncated ONNX fixed64 field"};
        } else if (wire == 2) {
            std::uint64_t length{};
            if (!read_varint(input, length) || !skip_bytes(input, length))
                return {false, 0, "truncated ONNX length-delimited field"};
        } else if (wire == 5) {
            if (!skip_bytes(input, 4)) return {false, 0, "truncated ONNX fixed32 field"};
        } else {
            return {false, 0, "unsupported ONNX protobuf wire type"};
        }
    }
    return {false, 0, "ONNX default opset is missing"};
}

ModelAssetLoadResult load_model_assets(const std::filesystem::path& manifest_path) {
    std::error_code error;
    const auto manifest_size = std::filesystem::file_size(manifest_path, error);
    if (error || manifest_size > 64U * 1024U) return {false, {}, "manifest is missing or too large"};
    std::ifstream input(manifest_path, std::ios::binary);
    if (!input) return {false, {}, "cannot open manifest"};
    std::map<std::string, std::string> fields;
    std::string line;
    while (std::getline(input, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (line.empty() || line.front() == '#') continue;
        const auto separator = line.find('=');
        if (separator == std::string::npos || separator == 0 || separator + 1 == line.size() ||
            !fields.emplace(line.substr(0, separator), line.substr(separator + 1)).second)
            return {false, {}, "invalid or duplicate manifest field"};
    }
    static const std::unordered_set<std::string> allowed{
        "manifest_version", "model_id", "architecture", "task", "format", "source_revision",
        "model_sha256", "vocabulary_sha256", "license", "model_file", "vocabulary_file",
        "onnx_opset", "input_ids_name", "attention_mask_name", "token_type_ids_name",
        "output_name", "input_element_type", "output_element_type", "output_columns",
        "maximum_sequence_length", "maximum_candidates"};
    if (fields.size() != allowed.size() ||
        !std::all_of(fields.begin(), fields.end(), [&](const auto& field) {
            return allowed.contains(field.first);
        })) return {false, {}, "manifest fields do not match version 1 schema"};

    ModelManifest manifest;
    std::size_t version{};
    std::size_t opset{};
    if (!parse_size(fields["manifest_version"], version) ||
        version > std::numeric_limits<std::uint32_t>::max() ||
        !parse_size(fields["onnx_opset"], opset) ||
        opset > std::numeric_limits<std::uint32_t>::max() ||
        !parse_size(fields["output_columns"], manifest.output_columns) ||
        !parse_size(fields["maximum_sequence_length"], manifest.maximum_sequence_length) ||
        !parse_size(fields["maximum_candidates"], manifest.maximum_candidates))
        return {false, {}, "invalid numeric manifest field"};
    manifest.manifest_version = static_cast<std::uint32_t>(version);
    manifest.model_id = fields["model_id"];
    manifest.architecture = fields["architecture"];
    manifest.task = fields["task"];
    manifest.format = fields["format"];
    manifest.source_revision = fields["source_revision"];
    manifest.model_sha256 = fields["model_sha256"];
    manifest.vocabulary_sha256 = fields["vocabulary_sha256"];
    manifest.license = fields["license"];
    manifest.model_file = fields["model_file"];
    manifest.vocabulary_file = fields["vocabulary_file"];
    manifest.onnx_opset = static_cast<std::uint32_t>(opset);
    manifest.input_ids_name = fields["input_ids_name"];
    manifest.attention_mask_name = fields["attention_mask_name"];
    manifest.token_type_ids_name = fields["token_type_ids_name"];
    manifest.output_name = fields["output_name"];
    manifest.input_element_type = fields["input_element_type"];
    manifest.output_element_type = fields["output_element_type"];
    const auto validation = validate_manifest(manifest);
    if (!validation.ok) return {false, {}, validation.diagnostic};

    const auto directory = std::filesystem::canonical(manifest_path, error).parent_path();
    if (error) return {false, {}, "cannot resolve manifest directory"};
    const auto model_path = std::filesystem::canonical(directory / manifest.model_file, error);
    if (error || model_path.parent_path() != directory)
        return {false, {}, "model file escapes manifest directory"};
    const auto vocabulary_path =
        std::filesystem::canonical(directory / manifest.vocabulary_file, error);
    if (error || vocabulary_path.parent_path() != directory)
        return {false, {}, "vocabulary file escapes manifest directory"};
    if (sha256_file(model_path, 512U * 1024U * 1024U) != manifest.model_sha256)
        return {false, {}, "model sha256 mismatch or file is unavailable"};
    if (sha256_file(vocabulary_path, 4U * 1024U * 1024U) != manifest.vocabulary_sha256)
        return {false, {}, "vocabulary sha256 mismatch or file is unavailable"};

    std::ifstream vocabulary_input(vocabulary_path, std::ios::binary);
    std::vector<std::string> vocabulary;
    while (std::getline(vocabulary_input, line)) {
        if (!line.empty() && line.back() == '\r') line.pop_back();
        if (line.empty()) return {false, {}, "vocabulary contains an empty token"};
        vocabulary.push_back(line);
        if (vocabulary.size() > 256U * 1024U) return {false, {}, "vocabulary has too many tokens"};
    }
    WordPieceTokenizer tokenizer(vocabulary);
    const auto tokenizer_validation = tokenizer.validation();
    if (!tokenizer_validation.ok) return {false, {}, tokenizer_validation.diagnostic};
    return {true, {std::move(manifest), model_path, vocabulary_path, std::move(vocabulary)}, {}};
}

WordPieceTokenizer::WordPieceTokenizer(std::vector<std::string> vocabulary,
                                       const bool lowercase_ascii)
    : vocabulary_(std::move(vocabulary)), lowercase_ascii_(lowercase_ascii) {
    if (vocabulary_.empty() || vocabulary_.size() >
                                   static_cast<std::size_t>(std::numeric_limits<std::int64_t>::max())) {
        validation_ = {false, "invalid vocabulary size"};
        return;
    }
    for (std::size_t index = 0; index < vocabulary_.size(); ++index) {
        if (vocabulary_[index].empty() ||
            !token_ids_.emplace(vocabulary_[index], static_cast<std::int64_t>(index)).second) {
            validation_ = {false, "vocabulary contains an empty or duplicate token"};
            return;
        }
    }
    for (const auto* required : {"[PAD]", "[UNK]", "[CLS]", "[SEP]"}) {
        if (!token_ids_.contains(required)) {
            validation_ = {false, std::string("vocabulary is missing ") + required};
            return;
        }
    }
    validation_ = {true, {}};
}

ValidationResult WordPieceTokenizer::validation() const { return validation_; }

std::int64_t WordPieceTokenizer::pad_token_id() const {
    const auto found = token_ids_.find("[PAD]");
    return found == token_ids_.end() ? -1 : found->second;
}

TokenizeResult WordPieceTokenizer::encode(const std::string_view text,
                                          const std::size_t maximum_sequence_length) const {
    if (!validation_.ok) return {false, {}, validation_.diagnostic};
    if (maximum_sequence_length < 2 || maximum_sequence_length > 512)
        return {false, {}, "invalid maximum sequence length"};

    std::vector<std::string> words;
    std::string ascii_word;
    auto flush_ascii = [&] {
        if (!ascii_word.empty()) words.push_back(std::move(ascii_word));
        ascii_word.clear();
    };
    for (std::size_t offset = 0; offset < text.size();) {
        const auto byte = static_cast<unsigned char>(text[offset]);
        if (byte <= 0x7f) {
            ++offset;
            if (std::isspace(byte) != 0) {
                flush_ascii();
            } else if (ascii_punctuation(byte)) {
                flush_ascii();
                words.emplace_back(1, static_cast<char>(byte));
            } else {
                auto character = static_cast<char>(byte);
                if (lowercase_ascii_) character = static_cast<char>(std::tolower(byte));
                ascii_word.push_back(character);
            }
            continue;
        }
        flush_ascii();
        std::string scalar;
        if (!decode_scalar(text, offset, scalar)) return {false, {}, "input is not valid UTF-8"};
        words.push_back(std::move(scalar));
    }
    flush_ascii();

    TokenizedInput output;
    output.tokens.push_back("[CLS]");
    for (const auto& word : words) {
        std::size_t start = 0;
        std::vector<std::string> pieces;
        while (start < word.size()) {
            std::size_t end = word.size();
            std::string matched;
            while (end > start) {
                auto piece = word.substr(start, end - start);
                if (start != 0) piece.insert(0, "##");
                if (token_ids_.contains(piece)) {
                    matched = std::move(piece);
                    break;
                }
                --end;
                while (end > start &&
                       (static_cast<unsigned char>(word[end]) & 0xc0U) == 0x80U) --end;
            }
            if (matched.empty()) {
                pieces.assign(1, "[UNK]");
                break;
            }
            pieces.push_back(std::move(matched));
            start = end;
        }
        output.tokens.insert(output.tokens.end(), pieces.begin(), pieces.end());
        if (output.tokens.size() + 1 > maximum_sequence_length)
            return {false, {}, "tokenized input exceeds maximum sequence length"};
    }
    output.tokens.push_back("[SEP]");
    output.input_ids.reserve(output.tokens.size());
    for (const auto& token : output.tokens) output.input_ids.push_back(token_ids_.at(token));
    output.attention_mask.assign(output.input_ids.size(), 1);
    output.token_type_ids.assign(output.input_ids.size(), 0);
    return {true, std::move(output), {}};
}

TokenizeResult WordPieceTokenizer::encode_pair(const std::string_view first,
                                               const std::string_view second,
                                               const std::size_t maximum_sequence_length) const {
    const auto left = encode(first, maximum_sequence_length);
    if (!left.ok) return left;
    const auto right = encode(second, maximum_sequence_length);
    if (!right.ok) return right;
    if (left.value.input_ids.size() + right.value.input_ids.size() - 1 > maximum_sequence_length)
        return {false, {}, "tokenized pair exceeds maximum sequence length"};
    TokenizedInput output = left.value;
    output.tokens.insert(output.tokens.end(), right.value.tokens.begin() + 1,
                         right.value.tokens.end());
    output.input_ids.insert(output.input_ids.end(), right.value.input_ids.begin() + 1,
                            right.value.input_ids.end());
    output.attention_mask.insert(output.attention_mask.end(), right.value.attention_mask.begin() + 1,
                                 right.value.attention_mask.end());
    output.token_type_ids.insert(output.token_type_ids.end(), right.value.input_ids.size() - 1, 1);
    return {true, std::move(output), {}};
}

}  // namespace owo::model
