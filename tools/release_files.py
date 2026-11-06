#!/usr/bin/env python3
"""Release files that are the same bytes on every build (SPEC-9.7.1, docs/RELEASING.md).

    release_files.py zip <dir> <out.zip>
        Every regular file under <dir> by its relative path with "/", sorted, deflated at
        level 9, all with the time SOURCE_DATE_EPOCH (UTC; 1980-01-01, zip's first date, when
        unset or earlier), mode 0755 for the binaries (names starting with "vy-") and 0644 for
        the rest, no directory entries and no extra fields. The same files give the same zip
        with the same Python (zlib).
    release_files.py sums <prefix> <file>...
        "<sha256>  <prefix>/<name>" per file, sorted by name: what `sha256sum -c` reads.
    release_files.py build-md <template> <fragment>...
        BUILD.md from tools/BUILD.md.in and one fragment per target ("key=value" lines that
        release.yml writes): @TAG@, @COMMIT@, @EPOCH@ and @DATE@ from the first fragment,
        @MACHINES@ and @PACKAGES@ as tables over all of them.

Standard library only; runs with the python3 of every GitHub runner.
"""

import hashlib
import os
import sys
import time
import zipfile

ZIP_EPOCH = 315532800  # 1980-01-01T00:00:00Z


def epoch():
    value = int(os.environ.get("SOURCE_DATE_EPOCH") or ZIP_EPOCH)
    return max(value, ZIP_EPOCH)


def write_zip(src, out):
    date_time = time.gmtime(epoch())[:6]
    names = []
    for parent, dirs, files in os.walk(src):
        rel = os.path.relpath(parent, src)
        for f in files:
            if os.path.isfile(os.path.join(parent, f)):
                names.append(f if rel == "." else "/".join(rel.split(os.sep) + [f]))
    names.sort()
    if not names:
        sys.exit(f"error: no files in {src}")
    with zipfile.ZipFile(out, "w") as z:
        for name in names:
            info = zipfile.ZipInfo(name, date_time)
            info.create_system = 3  # Unix: unzip reads the mode below
            info.create_version = 20
            info.extract_version = 20
            mode = 0o100755 if name.rpartition("/")[2].startswith("vy-") else 0o100644
            info.external_attr = mode << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            with open(os.path.join(src, *name.split("/")), "rb") as f:
                z.writestr(info, f.read(), compresslevel=9)


def sha256(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def sums(prefix, files):
    lines = []
    for path in sorted(files, key=os.path.basename):
        name = os.path.basename(path)
        lines.append(f"{sha256(path)}  {prefix}/{name}" if prefix else f"{sha256(path)}  {name}")
    sys.stdout.buffer.write(("\n".join(lines) + "\n").encode())


def read_fragment(path):
    out = {}
    with open(path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\r\n")
            if line:
                key, _, value = line.partition("=")
                out[key] = value
    return out


def build_md(template, fragments):
    machines = [read_fragment(p) for p in fragments]
    if not machines:
        sys.exit("error: no fragments")
    machines.sort(key=lambda m: m["target"])
    first = machines[0]
    rows = ["| Mål | Runner | Avbild | rustc | cargo | Signerad |", "| --- | --- | --- | --- | --- | --- |"]
    for m in machines:
        rows.append(
            f"| `{m['target']}` | {m['runner']} | {m['image']} | {m['rustc']} | {m['cargo']} | {m['signed']} |"
        )
    packages = ["| Mål | `studio.vyp`, paketidentitet |", "| --- | --- |"]
    for m in machines:
        packages.append(f"| `{m['target']}` | `{m['studio_identity']}` |")
    stamp = int(first["epoch"])
    with open(template, encoding="utf-8") as f:
        text = f.read()
    for key, value in [
        ("@TAG@", first["tag"]),
        ("@VERSION@", first["tag"].removeprefix("v")),
        ("@COMMIT@", first["commit"]),
        ("@EPOCH@", str(stamp)),
        ("@DATE@", time.strftime("%Y-%m-%d %H:%M:%S UTC", time.gmtime(stamp))),
        ("@MACHINES@", "\n".join(rows)),
        ("@PACKAGES@", "\n".join(packages)),
    ]:
        text = text.replace(key, value)
    sys.stdout.buffer.write(text.encode())


def main(argv):
    if len(argv) == 4 and argv[1] == "zip":
        write_zip(argv[2], argv[3])
    elif len(argv) >= 4 and argv[1] == "sums":
        sums(argv[2], argv[3:])
    elif len(argv) >= 4 and argv[1] == "build-md":
        build_md(argv[2], argv[3:])
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
