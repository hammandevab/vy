#!/usr/bin/env python3
"""Spec-spårning (SPEC-12.1.1): varje SPEC-id i SPEC.md måste refereras av minst en
fil under crates/, modules/, conformance/, tools/ eller .github/.

Avslutar med 1 och listar ospårade id:n annars. Kör: python3 tools/spec_trace.py
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ID = re.compile(r"SPEC-(?:\d+|[A-Z])\.\d+\.\d+")
SCAN_DIRS = ["crates", "modules", "conformance", "tools", ".github"]
SCAN_EXT = {".rs", ".py", ".sh", ".yml", ".yaml", ".md", ".json", ".toml"}


def main() -> int:
    spec = (ROOT / "SPEC.md").read_text(encoding="utf-8")
    declared = []
    seen = set()
    for m in re.finditer(r"\*\*\[(" + ID.pattern + r")\]\*\*", spec):
        sid = m.group(1)
        if sid in seen:
            print(f"duplicate id in SPEC.md: {sid}")
            return 1
        seen.add(sid)
        declared.append(sid)

    referenced: dict[str, set[str]] = {}
    for d in SCAN_DIRS:
        base = ROOT / d
        if not base.exists():
            continue
        for path in base.rglob("*"):
            if not path.is_file() or path.suffix not in SCAN_EXT:
                continue
            if "target" in path.parts:
                continue
            if path == ROOT / "SPEC.md":
                continue
            text = path.read_text(encoding="utf-8", errors="replace")
            for m in ID.finditer(text):
                referenced.setdefault(m.group(0), set()).add(str(path.relative_to(ROOT)))

    missing = [sid for sid in declared if sid not in referenced]
    unknown = sorted(sid for sid in referenced if sid not in seen)

    print(f"{len(declared)} id:n i SPEC.md, {len(declared) - len(missing)} spårade")
    for sid in unknown:
        print(f"  refererat men saknas i SPEC.md: {sid} ({', '.join(sorted(referenced[sid]))})")
    for sid in missing:
        print(f"  ospårat: {sid}")
    return 1 if (missing or unknown) else 0


if __name__ == "__main__":
    sys.exit(main())
