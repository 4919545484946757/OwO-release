#include "owo/engine/binary_lexicon.h"

int main(int argc, char** argv) {
    if (argc != 2) return 2;
    owo::engine::BinaryLexicon lexicon;
    if (!lexicon.load(argv[1]).success || lexicon.size() != 2) return 1;
    const std::string_view xie[]{"yi", "xie"};
    const std::string_view xue[]{"yi", "xue"};
    return lexicon.lookup(xie).size() == 1 && lexicon.lookup(xue).size() == 1 ? 0 : 1;
}
