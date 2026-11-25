# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Test docs/hooks/front_matter.py."""

# Run from the repository root: python3 -I -B docs/hooks/test_front_matter.py .

from __future__ import annotations

import importlib
import sys
import types
from pathlib import Path

from mkdocs.utils.meta import get_data

ROOT = Path(sys.argv[1]).resolve()
sys.path.append(str(ROOT / "docs/hooks"))
hooktest = importlib.import_module("hooktest")
MOD = hooktest.load_hook(ROOT, "front_matter")

# (case name, raw page source, warnings).
CASES: list[tuple[str, str, int]] = [
    ("read front matter, block stripped", "---\ntitle: Title\n---\n\n# Title\n", 0),
    (
        "valid metadata then thematic breaks",
        "---\ntitle: T\n---\n\n---\n\nIntro\n\n---\n",
        0,
    ),
    ("valid metadata and thematic-break body", "---\ntitle: T\n---\n---\nx\n---\n", 0),
    ("file unreadable", "# Title\n", 1),
    ("no front matter", "# Title\n\ntext\n", 0),
    (
        "unquoted colon in value",
        "---\ndescription: Tap of the organisation: formulae.\n---\n\n# T\n",
        1,
    ),
    ("block is a string", "---\njust text\n---\n\n# T\n", 1),
    ("invalid date", "---\ndate: 2026-13-45\n---\n\n# T\n", 1),
    ("empty block", "---\n\n---\n\n# T\n", 1),
    ("no closing line", "---\ndescription: a: b\n\n# T\n", 0),
    ("thematic break after a heading", "# T\n\n---\n\ntext\n---\n", 0),
]


class _UnreadableFile:
    """Represent a page whose source cannot be read."""

    src_uri = "index.md"
    src_path = "index.md"

    @property
    def content_string(self) -> str:
        """Raise the source read error."""
        raise OSError("unreadable")


def _run_case(name: str, src: str, want_warns: int) -> bool:
    """Pass one page through the hook and compare the warning count."""
    # git-revision-date-localized fills page.meta before hooks run.
    file = (
        _UnreadableFile()
        if name == "file unreadable"
        else types.SimpleNamespace(
            src_uri="index.md", src_path="index.md", content_string=src
        )
    )
    page = types.SimpleNamespace(
        meta={"git_revision_date_localized": "2026-11-18"},
        file=file,
    )
    markdown = get_data(src)[0]
    with hooktest.capture_warnings("mkdocs.hooks.front_matter") as handler:
        out = MOD.on_page_markdown(markdown, page, {}, None)
    ok = out == markdown and len(handler.messages) == want_warns
    detail = "" if ok else f"-> warns={handler.messages!r} changed={out != markdown}"
    print("ok  " if ok else "FAIL", name, detail)
    return ok


def main() -> int:
    """Run all front-matter cases and exit 1 on any failure."""
    bad = 0
    for name, src, want_warns in CASES:
        if not _run_case(name, src, want_warns):
            bad += 1
    print(f"cases={len(CASES)} failed={bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
