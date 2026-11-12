"""Extract the unchanged (after gunzip) English cBpack and its upstream notices."""

import argparse
import gzip
import hashlib
from pathlib import Path
import zipfile

WHEEL_SHA = "4b1c6ecffc6198be3396d5cf871c4423ca71c907c231348d352dd54d62b97473"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("wheel", type=Path)
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    with args.wheel.open("rb") as wheel:
        if hashlib.file_digest(wheel, "sha256").hexdigest() != WHEEL_SHA:
            parser.error("not the pinned PyPI wordfreq 3.1.1 wheel")
        with zipfile.ZipFile(wheel) as archive:
            data = gzip.decompress(archive.read("wordfreq/data/large_en.msgpack.gz"))
            info = "wordfreq-3.1.1.dist-info/"
            license_text = archive.read(info + "LICENSE.txt")
            metadata = archive.read(info + "METADATA").decode("utf-8")
            notices = metadata[metadata.index("\n## License\n") + 1 :]
    args.destination.mkdir(parents=True, exist_ok=True)
    (args.destination / "wordfreq-en.msgpack").write_bytes(data)
    (args.destination / "LICENSE.wordfreq").write_bytes(license_text)
    (args.destination / "NOTICE.wordfreq.md").write_bytes((notices.rstrip() + "\n").encode("utf-8"))
    print(hashlib.sha256(data).hexdigest(), len(data), "wordfreq-en.msgpack")


if __name__ == "__main__":
    main()
