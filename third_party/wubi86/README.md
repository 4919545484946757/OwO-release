# Wubi86 verification and conversion inputs

`sources/` preserves the 65 original files referenced by
`data/wubi86/downloads.json`, byte for byte (34,362,253 bytes total).
The manifest records each source URL, pinned version, SHA-256, license and use.
Preserve the original license notices; inclusion is not a blanket license grant.

The importer, root generator and integrity test read this directory instead of
the untracked `env/` download backup. `.gitattributes` disables text conversion
for these files so receipt hashes survive Git checkout on Windows.

Production data remains in `data/wubi86/`; the runtime does not read these raw
sources. No production dictionary was regenerated during the migration.

Validation after migration: Debug and Release Wubi86 CTest groups each passed
6/6. The integrity verifier and all conversion input hashes also passed with
Python file reads beneath `env/` explicitly forbidden.
