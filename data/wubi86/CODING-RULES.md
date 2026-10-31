# Wubi86 root selection and code rules

This module uses the explicit character full-code table, rather than guessing a decomposition from Unicode names. The graphical inventory contains standard roots and variants, with source shapes and keys preserved. A font source slot is an asset address, not a new character encoding. See SOURCES.md and ROOTS-NOTICE.md.

Root selection follows writing order, largest valid root first, visual structure, separated before connected before intersecting. Those principles alone do not resolve all exceptional characters; do not mechanically split an unknown character or infer a code from its longest lookup alias. Supplied full-code records decide exceptional cases.

- Key-name characters: repeat the corresponding key four times (e.g. 工/aaaa, 人/wwww).
- Character-forming roots: root key followed by its first, second and last stroke codes; a two-stroke character has only its available strokes after the root key (e.g. 丁/sgh).
- Basic one-stroke characters use their source-defined special full codes (e.g. 一/ggll, 乙/nnll); these are explicit records, not inferred padding for other characters.
- Ordinary characters: four roots take four keys; more than four roots take first, second, third and last. Two/three roots add the last-stroke/structure identifier.
- Identifier keys by stroke (left-right / top-bottom / other): horizontal g/f/d; vertical h/j/k; left-falling t/r/e; dot/right-falling y/u/i; fold n/b/v. Stroke/structure exceptions must follow the source record; this table is not permission to guess them.
- One-, two-, three-key short forms are independently listed by the short-form source. A three-key record can itself be a full code (好/vbg); a short form need not prefix its full code (我/q versus trnt).
- Phrases use full single-character codes only: 2+2, 1+1+2, or first/second/third/last first keys. No automatic commit, learning, UI or plugin behavior is implemented.

Source basis: pinned explicit 86 full-code resource; pinned Jidian encoder formula declarations and main-table examples; reference 86 root chart; existing publisher excerpts for background. These are functional rule descriptions, not a copied tutorial or a new decomposition dataset.
