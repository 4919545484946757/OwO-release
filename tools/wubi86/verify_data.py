"""Read-only validation of all generated assets, raw download receipts and GB2312 map."""
from pathlib import Path
import hashlib
import json
from collections import Counter
ROOT = Path(__file__).resolve().parents[2]

def verify():
    for name, expected in json.loads((ROOT/"data/wubi86/output-sha256.json").read_text(encoding="utf-8")).items():
        if hashlib.sha256((ROOT/name).read_bytes()).hexdigest() != expected:
            raise ValueError(f"generated asset hash mismatch: {name}")
    downloads=json.loads((ROOT/"data/wubi86/downloads.json").read_text(encoding="utf-8"))["downloads"]
    for item in downloads:
        path=ROOT/item["path"]
        if not path.resolve().is_relative_to((ROOT/"env/wubi86").resolve()):
            raise ValueError("download outside env/wubi86")
        data=path.read_bytes()
        if len(data)!=item["bytes"] or hashlib.sha256(data).hexdigest()!=item["sha256"]:
            raise ValueError(f"raw download receipt mismatch: {path}")
    expected=set()
    for a in range(0xb0,0xf8):
        for b in range(0xa1,0xff):
            try: expected.add((bytes([a,b]).decode("gb2312"),f"{a:02X}{b:02X}"))
            except UnicodeDecodeError: pass
    got={tuple(line.split("\t")) for line in (ROOT/"data/wubi86/gb2312.tsv").read_text(encoding="utf-8").splitlines()}
    if len(expected)!=6763 or got!=expected:
        raise ValueError("GB2312 position map mismatch")
    source=json.loads((ROOT/"data/wubi86/roots.json").read_text(encoding="utf-8"))
    if len(source["roots"])!=257 or Counter(r["key"] for r in source["roots"])!=source["key_counts"]:
        raise ValueError("root coverage mismatch")
    print(f"PASS generated assets, {len(downloads)} raw receipts, GB2312 exact position map, 257 root identities")

if __name__=="__main__":
    verify()
