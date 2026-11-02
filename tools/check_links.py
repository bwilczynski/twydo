#!/usr/bin/env python3
"""Report relative Markdown links that point at files which do not exist.

Usage: check_links.py [ROOT]   (default: the repository root)

Prints `path:line: broken link <target>` per broken link and exits 1 if there are any.
External links (any URL scheme), pure anchors and links inside fenced code blocks are skipped.
"""

import re
import sys
from pathlib import Path
from urllib.parse import unquote

SKIP_DIRS = {"target", ".git", "node_modules", ".superpowers"}
LINK = re.compile(r"\]\(([^)\s]+)(?:\s+\"[^\"]*\")?\)")
CODE_SPAN = re.compile(r"(`+).*?\1")
SCHEME = re.compile(r"^[a-zA-Z][a-zA-Z0-9+.-]*:")


def markdown_files(root: Path):
    for path in sorted(root.rglob("*.md")):
        if not SKIP_DIRS.intersection(path.relative_to(root).parts[:-1]):
            yield path


def broken_links(root: Path) -> list[tuple[Path, int, str]]:
    broken = []
    for path in markdown_files(root):
        in_fence = False
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.lstrip().startswith(("```", "~~~")):
                in_fence = not in_fence
                continue
            if in_fence:
                continue
            for target in LINK.findall(CODE_SPAN.sub("", line)):
                if SCHEME.match(target) or target.startswith("#"):
                    continue
                file_part = unquote(target.split("#", 1)[0])
                if not (path.parent / file_part).exists():
                    broken.append((path.relative_to(root), number, target))
    return broken


def main(argv: list[str]) -> int:
    root = Path(argv[0]) if argv else Path(__file__).resolve().parent.parent
    broken = broken_links(root)
    for path, number, target in broken:
        print(f"{path}:{number}: broken link {target}")
    return 1 if broken else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
