# Wubi86 Phase 1 implementation plan

Approved specification: user request in this chat, 2026-10-30.
Architecture: independent C++20 owo::wubi86 / owo_wubi86; no engine or runtime integration.

- [x] Pin and audit upstream main table, retain downloads under env/wubi86 only; record hashes and license.
- [x] Write executable tests first, register CTest, confirm missing implementation fails.
- [x] Implement strict UTF-8/types, explicit code kinds, transactional dictionary loader, subtree-only Trie, table-based encoder.
- [x] Normalize upstream records reproducibly; quarantine malformed records, never infer full codes from maximum length.
- [x] Compile and run Wubi tests and all owo.engine tests; run available static checks and diff audit.
- [x] Deliver evidence and distinguish library functionality, verified data coverage, and production readiness.

Ruling: upstream lacks explicit full/short classification. Unclassified records stay queryable but cannot encode phrases. Only explicit reviewed full-code records may feed the encoder. Missing data is reported, not guessed.
Ruling: no automatic plugin integration or commits; no installation or environment changes.

Previous-pass outcome (superseded by completion pass): Phase 1 acceptance was NOT met: reviewed full-code/root data are partial and one pre-existing engine regression test fails on both working tree and unmodified HEAD.

## Completion pass (2026-10-30)
- Import explicit-position full-code source (MIT), verify all GB2312 via existing Python codec; audit conflicts against Jidian and Rime, never longest-code infer.
- Preserve vetted alternatives; separate source short codes and isolate unverified records. Validate every phrase against full-code formulas.
- Add all graphical roots from an APL-licensed glyph source with stable IDs, exact paths, source slots and key positions; keep modified glyph slots distinct from Unicode meanings.
- Extend, do not remove, mechanics tests: full-core coverage, all query rows, all prefixes, phrase cross-checks and timings.
- Rebuild, run Wubi/Engine/baseline, MSVC /analyze, regenerate deterministically, update report and downloads.

Completion-pass outcome: core 6763/6763, complete reference glyph inventory 257/25 keys, verified production dictionary 87694 records, Wubi 3/3 PASS, Engine 15/16 with identical baseline failure, MSVC /analyze PASS. See REPORT.md for exact scope and extension limitations.
