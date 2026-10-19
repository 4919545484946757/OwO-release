"""Package one plugin directory into an .owopkg (ZIP) with forward-slash entry names.

PowerShell 5.1's Compress-Archive writes backslash-separated central-directory
names, which the plugin installer's safe_package_path rejects. This script is
the replacement: it always emits '/' separators and UTF-8 names.

usage: python package_plugin_zip.py <source-directory> <output.owopkg>
"""
import sys
import zipfile
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        print("usage: package_plugin_zip.py <source-directory> <output.owopkg>", file=sys.stderr)
        return 2
    source = Path(sys.argv[1]).resolve()
    output = Path(sys.argv[2])
    if not source.is_dir():
        print(f"source directory not found: {source}", file=sys.stderr)
        return 2
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_STORED) as archive:
        for file in sorted(source.rglob("*")):
            if not file.is_file():
                continue
            relative = file.relative_to(source).as_posix()
            archive.write(file, relative)
    return 0


if __name__ == "__main__":
    sys.exit(main())
