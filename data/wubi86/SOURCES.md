# Wubi86 production data provenance

## Inputs and exact format

All raw inputs live only under env/wubi86. input-lock.json pins every file consumed by conversion; downloads.json lists every download (including rejected research sources), URL, revision, size, SHA-256, license and actual use.

1. KyleBing/rime-wubi86-jidian commit `4d08110b8080f6577dad733ada7e5ae5c5e878fd`, main table version 4.3, MIT, Copyright (c) Kyle Bing. Only the main body is read; no YAML imports, extensions, user dictionaries or pinyin are followed. Main-table columns: text, code, optional weight, optional stem. The populated stem values are annotations, not reliable full-code definitions.
2. sfyc23/python-wubi commit `42c6ac6b4d2285acc7f39594274a4644cdd358ff`, MIT. `resource/86版_全码.txt` explicitly supplies code point, character, full code, then source short forms. Import uses this positional full-code field, NOT longest-code or code-length inference. It has 21,004 rows. 80 private-use slots U+E815–U+E864 are excluded, leaving 20,924 portable characters. Source short forms can differ from full-code prefixes: 我/q, 不/i, 以/c, 为/o, 有/e, 这/p.
3. rime/rime-wubi pinned in downloads.json, LGPL-3.0: cross-check only; no Rime data or software is imported into production. CNMan has no verified explicit data license; viewer third-party decomposition licensing is unresolved. Neither is imported.

## Full-code validation and policy

Every GB2312 character was enumerated using the existing Python standard-library gb2312 codec at rows B0–F7 and columns A1–FE; exact assigned positions total 6,763. gb2312.tsv preserves every byte position. 6,762 core full codes have Jidian lookup witnesses after explicit corrections. The remaining 式/aad is in the explicit MIT full-code source (Jidian only has its short code aa); it follows 工+弋+identifier d. No core character is missing.

The full source has two core mistakes: 发/ntyc -> ntcy and 靥/dddd -> dddl. full-code-corrections.json records each original, replacement and independent Jidian/Rime witnesses; the importer verifies these witnesses before applying corrections. 麸/gqfw and 麴/fwwo follow agreement between the explicit full table and Jidian; differing Rime values are not imported. The older audited alternative 戈/agny is retained alongside the explicit source's agnt. No other arbitrary four-key alias is promoted. Example 艹/copp remains quarantined; canonical 艹/aghh is usable.

Result: 20,925 full-code records for 20,924 characters, 4,183 source short-code records, 62,586 verified phrase records = 87,694 production rows. Core coverage is 6,763/6,763. Outside core: 14,161 characters (14,139 additional CJK unified ideographs, 21 compatibility ideographs, 〇). Total CJK unified characters: 20,902. This is not an all-Unicode-CJK encoding claim. All unconfirmed aliases and out-of-source characters remain quarantined/unsupported; no mappings are invented. Extension codes are source-conformant, not individually certified by a standards authority.

The original main table has 89,260 records before validation: two malformed records excluded; three duplicates merged -> 89,255 unique records. All were examined. Of those, 1,572 are quarantined: 1,504 unverified character aliases, 33 phrases with unsupported characters (including Latin/punctuation), 35 phrase mismatches. The remaining 87,683 source pairs plus 11 missing explicit full-code records with neutral weight 0 form the production dictionary. All additions are listed in normalization-audit.json. Five missing weights explicitly become 0. Existing weights are preserved, and duplicate weights use max. Weight means upstream candidate preference, not measured corpus counts.

## Phrase policy

Generate from verified full codes only: 2+2 for two characters; 1+1+2 for three; first/second/third/last first keys for four or more. All characters must have verified full codes, including unselected middle characters. Enumerate all reviewed full-code alternatives, deduplicate fragments/results, sort lexicographically; exceeding the configured limit is an error, never silent truncation. All 62,586 admitted phrases exactly match generated codes. Nonmatching legacy shortcuts (e.g. some 足旁 compounds), mixed Latin and unsupported entries are retained in quarantine.json with their original weights and expected results, not silently relabeled or rewritten.

## Graphical roots

riku-ri/wubi-kb revision is pinned in downloads.json and input-lock.json; the separate font pre-release binary is SHA-pinned. The derivative UMingCN-wubi-86-kb.ttf embeds Arphic Public License 1999. Its exact license, original copyright and upstream modification notice are retained as LICENSE.roots, ROOTS-NOTICE.md and font-modifications.adoc. FontTools was already installed and was used as a library; no font/software was installed. No TeX code was copied or executed. Factual key/glyph assignments were extracted from the 25 reference chart sections; outlines were converted verbatim from the font.

The complete reference inventory has 257 glyph forms over 25 keys. Each shape has a stable W86-key-index identity, key assignment, source font slot and exact SVG path. Modified font slots are NOT Unicode meanings: e.g. W86-g-02 uses source slot U+9752 for 青字头, never pretends the whole 青 is a root. Layout padding 〇 in L/O TeX groups is excluded. All forms and variants are available through Wubi86CodeTable::root_glyphs/root_glyph and roots.json; roots-chart.svg allows independent visual checking without a font installation or application UI. The prior root_key Unicode convenience API remains for unambiguous named roots. The count includes variants; it is not a claim that 257 is the number of distinct traditional basic roots.

## Reproduction

Use existing tools only:

```
python tools/wubi86/normalize.py
python tools/wubi86/generate_roots.py
python tools/wubi86/verify_data.py
cmake --preset windows-debug
cmake --build --preset windows-debug --target owo_wubi86_tests owo_wubi86_coverage_tests
ctest --test-dir build/windows-debug -C Debug -R "^owo.wubi86" --output-on-failure
```

No network calls occur in these scripts. Hash mismatch, malformed explicit input, missing core coverage or correction-witness mismatch fails conversion. output-sha256.json records generated artifacts for read-only integrity checks. The source files are preserved under env; production files are generated locally.

## Golden tests and scope

Existing mechanical tests are retained. Their 甲/乙/丙 fixtures are synthetic and never used to certify standard codes. Previous negative expectations for unclassified 中/艹 were strengthened to positive canonical-code checks while retaining rejection of copp. New Golden checks include short full codes 中/khk、好/vbg、天/gdi; corrected 发/ntcy、靥/dddl; 式/aad; source variants 戈/agnt,agny and non-prefix short 我/q. The explicit full source supplies expectations; corrected pairs also require two fixed witnesses. All GB2312 code lookups, all production exact buckets, all nonempty prefixes, frequency ordering and all 62,586 phrase encodings are actually tested. No claim of arbitrary character image recognition or automatic glyph segmentation is made: Phase 1 single-character encoding is a verified table lookup.
