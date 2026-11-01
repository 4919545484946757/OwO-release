"""Offline, source-pinned Wubi86 import. Never infer full codes by length.
The full-table resource explicitly supplies canonical full code in column 3,
followed by source short codes. All uncertain original records are quarantined.
"""
from pathlib import Path
from collections import Counter, defaultdict
from itertools import product
import hashlib
import json
import re
import shutil

ROOT = Path(__file__).resolve().parents[2]
SOURCES = ROOT / "third_party/wubi86/sources"
OUT = ROOT / "data/wubi86"
COMMIT = "4d08110b8080f6577dad733ada7e5ae5c5e878fd"
PYCOMMIT = "42c6ac6b4d2285acc7f39594274a4644cdd358ff"


def gb2312_characters():
    chars = []
    for high in range(0xb0, 0xf8):
        for low in range(0xa1, 0xff):
            try:
                chars.append((bytes([high, low]).decode("gb2312"), f"{high:02X}{low:02X}"))
            except UnicodeDecodeError:
                pass
    if len(chars) != 6763 or len(set(c for c, _ in chars)) != 6763:
        raise ValueError("GB2312 codec coverage mismatch")
    return chars


def read_jidian():
    source = (SOURCES / "wubi86_jidian.dict.yaml").read_text(encoding="utf-8-sig")
    header, body = source.split("...\n", 1)
    if 'version: "4.3"' not in header:
        raise ValueError("unexpected main-table version")
    rows = {}
    audit = []
    duplicates = 0
    for line_no, line in enumerate(body.splitlines(), len(header.splitlines()) + 2):
        if not line or line.startswith("#"):
            continue
        f = line.split("\t")
        if len(f) not in (2, 3, 4) or not f[0] or not re.fullmatch("[a-y]{1,4}", f[1]):
            audit.append(dict(line=line_no, record=line, reason="malformed code/text/columns"))
            continue
        weight = f[2] if len(f) >= 3 else "0"
        if not re.fullmatch("[0-9]+", weight) or int(weight) > 0xffffffff:
            raise ValueError(f"invalid weight at line {line_no}")
        if len(f) == 2:
            audit.append(dict(line=line_no, record=line, reason="missing weight explicitly normalized to zero"))
        if len(f) == 4:
            audit.append(dict(line=line_no, record=line, reason="stem is annotation, retained here, not interpreted as code"))
        key = f[0], f[1]
        if key in rows:
            duplicates += 1
            audit.append(dict(line=line_no, record=line, reason="duplicate: retain max weight"))
        rows[key] = max(rows.get(key, 0), int(weight))
    return rows, audit, duplicates


def phrase_codes(text, full):
    if len(text) < 2 or any(c not in full for c in text):
        return set()
    if len(text) == 2:
        parts = [(text[0], 2), (text[1], 2)]
    elif len(text) == 3:
        parts = [(text[0], 1), (text[1], 1), (text[2], 2)]
    else:
        parts = [(text[0], 1), (text[1], 1), (text[2], 1), (text[-1], 1)]
    return {"".join(p) for p in product(*[{code[:n] for code in full[c] if len(code) >= n} for c, n in parts])}


def normalize():
    lock = json.loads((OUT / "input-lock.json").read_text(encoding="utf-8"))
    for name, digest in lock.items():
        if hashlib.sha256((SOURCES / name).read_bytes()).hexdigest() != digest:
            raise ValueError(f"source hash mismatch: {name}")
    records, audit, duplicates = read_jidian()
    full, short = {}, {}
    source_exclusions = []
    corrections = json.loads((OUT / "full-code-corrections.json").read_text(encoding="utf-8"))
    rime_pairs = set()
    for line in (SOURCES / "rime-wubi86.dict.yaml").read_text(encoding="utf-8-sig").split("...\n", 1)[1].splitlines():
        f = line.split("\t")
        if len(f) >= 2 and not line.startswith("#"):
            rime_pairs.add((f[0], f[1]))
    source_rows = (SOURCES / "pythonwubi-resource_86版_全码.txt").read_text(encoding="utf-8-sig").splitlines()
    for line in source_rows:
        f = line.split("\t")
        if len(f) < 3 or len(f[1]) != 1 or int(f[0], 16) != ord(f[1]) or f[1] in full:
            raise ValueError("invalid explicit full-code row")
        char, code = f[1], f[2]
        if 0xe000 <= ord(char) <= 0xf8ff:
            source_exclusions.append(dict(scalar=f"U+{ord(char):04X}", codes=f[2:], reason="private-use source slot has no portable character identity"))
            continue
        if any(not re.fullmatch("[a-y]{1,4}", c) for c in f[2:]):
            raise ValueError("invalid source code")
        if char in corrections:
            fix = corrections[char]
            if code != fix["original"] or (char, fix["full"]) not in records or (char, fix["full"]) not in rime_pairs:
                raise ValueError("correction lacks pinned witnesses")
            code = fix["full"]
        full[char] = {code}
        short[char] = set(f[3:])
        if any(len(c) >= len(code) for c in short[char]):
            raise ValueError("unexpected source short-code field")
    # Retain the independently reviewed source alternative, not arbitrary four-key aliases.
    reviewed = json.loads((OUT / "reviewed-full-codes.json").read_text(encoding="utf-8"))
    if reviewed["source_commit"] != COMMIT:
        raise ValueError("reviewed alternative source commit mismatch")
    for char, codes in reviewed["codes"].items():
        for code in codes:
            if (char, code) not in records:
                raise ValueError("reviewed alternative absent from pinned Jidian")
            full.setdefault(char, set()).add(code)
    core = gb2312_characters()
    missing = [c for c, _ in core if c not in full]
    if missing:
        raise ValueError(f"missing GB2312 full codes: {missing}")
    rows = {}
    quarantine = []
    phrase_stats = Counter()
    for (text, code), weight in records.items():
        if len(text) == 1:
            if code in full.get(text, set()):
                kind = "full"
            elif code in short.get(text, set()):
                kind = "short"
            else:
                quarantine.append(dict(text=text, code=code, weight=weight, reason="unverified character alias"))
                continue
        else:
            generated = phrase_codes(text, full)
            if code in generated:
                kind = "phrase"
            else:
                reason = "missing verified character" if not generated else "phrase code mismatch"
                quarantine.append(dict(text=text, code=code, weight=weight, reason=reason, generated=sorted(generated)))
                phrase_stats[reason] += 1
                continue
            phrase_stats[kind] += 1
        rows[code, text] = weight, kind
    # Add explicit single-character codes absent from lookup source with neutral weight.
    for char, codes in full.items():
        for code in codes:
            key = code, char
            rows[key] = records.get((char, code), 0), "full"
    for char, codes in short.items():
        for code in codes:
            key = code, char
            if key not in rows:
                rows[key] = records.get((char, code), 0), "short"
    additions = [dict(text=text, code=code, weight=weight, kind=kind) for (code, text), (weight, kind) in sorted(rows.items()) if (text, code) not in records]
    counts = Counter(kind for weight, kind in rows.values())
    counts.update(dict(records=len(rows), source_unique_records=len(records), source_duplicates=duplicates,
                       gb2312_covered=6763,
                       gb2312_level1=sum(int(position[:2], 16) <= 0xd7 for c, position in core),
                       gb2312_level2=sum(int(position[:2], 16) >= 0xd8 for c, position in core),
                       full_characters=len(full),
                       extension_characters=len(set(full) - {c for c, _ in core}),
                       excluded_private_use_characters=len(source_exclusions),
                       cjk_unified_characters=sum(0x4e00 <= ord(c) <= 0x9fff for c in full),
                       compatibility_characters=sum(0xf900 <= ord(c) <= 0xfaff for c in full),
                       ideographic_zero_characters=sum(c == "〇" for c in full),
                       multi_full_characters=sum(len(v) > 1 for v in full.values()), quarantined=len(quarantine)))
    header = f"OWO-WUBI86\t1\nversion\tJidian-4.3+pywubi@{PYCOMMIT}\nsource\tJidian@{COMMIT};pywubi@{PYCOMMIT};input-lock.json\n...\n"
    with (OUT / "main.tsv").open("w", encoding="utf-8", newline="\n") as output:
        output.write(header)
        for (code, text), (weight, kind) in sorted(rows.items()):
            output.write(f"{code}\t{text}\t{weight}\t{kind}\n")
    (OUT / "gb2312.tsv").write_text("".join(f"{char}\t{position}\n" for char, position in core), encoding="utf-8")
    (OUT / "full-codes.tsv").write_text("".join(f"{char}\t{'/'.join(sorted(codes))}\n" for char, codes in sorted(full.items())), encoding="utf-8")
    (OUT / "quarantine.json").write_text(json.dumps(quarantine, ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    # Exact collisions (one code with several distinct candidate texts).
    by_code = Counter(code for code, text in rows)
    counts["collision_codes"] = sum(n > 1 for n in by_code.values())
    counts["collision_records"] = sum(n for n in by_code.values() if n > 1)
    full_collisions = Counter(code for (code, text), (weight, kind) in rows.items() if kind == "full")
    counts["single_full_collision_codes"] = sum(n > 1 for n in full_collisions.values())
    counts["single_full_collision_records"] = sum(n for n in full_collisions.values() if n > 1)
    for length in (3, 4):
        counts[f"full_code_length_{length}"] = sum(kind == "full" and len(code) == length for (code, text), (weight, kind) in rows.items())
    counts["core_jidian_full_disagreements_after_corrections"] = sum(not any((c, code) in records for code in full[c]) for c, _ in core)
    (OUT / "normalization-audit.json").write_text(json.dumps(dict(counts=dict(counts), phrase_validation=dict(phrase_stats), source_additions=additions, corrections=corrections, excluded_source_characters=source_exclusions, exceptions=audit), ensure_ascii=False, indent=2)+"\n", encoding="utf-8")
    shutil.copyfile(SOURCES / "LICENSE", OUT / "LICENSE.upstream")
    shutil.copyfile(SOURCES / "pythonwubi-LICENSE", OUT / "LICENSE.pywubi")
    print(json.dumps(dict(counts)))


if __name__ == "__main__":
    normalize()
