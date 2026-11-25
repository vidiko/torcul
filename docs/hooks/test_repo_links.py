# SPDX-FileCopyrightText: 2026 Lusoris <lusoris@pm.me>
# SPDX-License-Identifier: EUPL-1.2
"""Test docs/hooks/repo_links.py."""

# Run from the repository root: python3 -I -B docs/hooks/test_repo_links.py .

from __future__ import annotations

import importlib
import sys
import types
from html.parser import HTMLParser
from pathlib import Path
from typing import Any

import markdown

ROOT = Path(sys.argv[1]).resolve()
sys.path.append(str(ROOT / "docs/hooks"))
hooktest = importlib.import_module("hooktest")
MOD = hooktest.load_hook(ROOT, "repo_links")

CFG: dict[str, Any] = {
    "repo_url": "https://github.com/vidiko/alveus",
    "edit_uri": "edit/main/docs/",
}
TRACKED = frozenset(
    {
        "CLEANROOM.md",
        "LICENSE",
        "docs/index.md",
        "docs/plan/README.md",
        "docsx/y.md",
        "lua/alveus.lua",
    }
)
B = "https://github.com/vidiko/alveus/blob/main/"
T = "https://github.com/vidiko/alveus/tree/main/"
H = "https://github.com/vidiko/alveus/blob/HEAD/"

# Each entry: page URI, edit URI, source, expected hrefs, sources, warnings.
CASES: list[tuple[str, str, str, list[str], list[str], int]] = [
    (
        "index.md",
        "edit/main/docs/",
        "[a](../CLEANROOM.md)",
        [f"{B}CLEANROOM.md"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        "[a](../CLEANROOM.md#anchor)",
        [f"{B}CLEANROOM.md#anchor"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        "[a](../CLEANROOM.md?plain=1#x)",
        [f"{B}CLEANROOM.md?plain=1#x"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        '[a](../CLEANROOM.md "t")',
        [f"{B}CLEANROOM.md"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        "[a](<../CLEANROOM.md>)",
        [f"{B}CLEANROOM.md"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        "[b][r]\n\n[r]: ../CLEANROOM.md",
        [f"{B}CLEANROOM.md"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        "[first\nsecond](../CLEANROOM.md)",
        [f"{B}CLEANROOM.md"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        "[![b](assets/x.svg)](../CLEANROOM.md)",
        [f"{B}CLEANROOM.md"],
        ["assets/x.svg"],
        0,
    ),
    ("index.md", "edit/main/docs/", "`[a](../CLEANROOM.md)`", [], [], 0),
    (
        "index.md",
        "edit/main/docs/",
        "first ` paragraph\n\n[a](../CLEANROOM.md)\n\nsecond ` paragraph",
        [f"{B}CLEANROOM.md"],
        [],
        0,
    ),
    ("index.md", "edit/main/docs/", "```md\n[a](../CLEANROOM.md)\n```", [], [], 0),
    ("index.md", "edit/main/docs/", "~~~\n[a](../CLEANROOM.md)\n~~~", [], [], 0),
    (
        "index.md",
        "edit/main/docs/",
        "- item\n\n  ```\n  [a](../CLEANROOM.md)\n  ```",
        [],
        [],
        0,
    ),
    ("index.md", "edit/main/docs/", "    [a](../CLEANROOM.md)", [], [], 0),
    ("index.md", "edit/main/docs/", "[a](../lua/)", [f"{T}lua"], [], 0),
    (
        "index.md",
        "edit/main/docs/",
        "[a](../%43LEANROOM.md)",
        [f"{B}CLEANROOM.md"],
        [],
        0,
    ),
    ("index.md", "edit/main/docs/", "[a](../SECURITY.md)", ["../SECURITY.md"], [], 1),
    ("index.md", "edit/main/docs/", "[a](../NO-SUCH.md)", ["../NO-SUCH.md"], [], 1),
    (
        "index.md",
        "edit/main/docs/",
        "[a](https://example.org/x)",
        ["https://example.org/x"],
        [],
        0,
    ),
    ("index.md", "edit/main/docs/", "[a](#anchor)", ["#anchor"], [], 0),
    ("index.md", "edit/main/docs/", "[a](plan/README.md)", ["plan/README.md"], [], 0),
    ("index.md", "edit/main/docs/", "[a](../../x.md)", ["../../x.md"], [], 1),
    (
        "index.md",
        "edit/main/docs/",
        "[a](../)",
        ["https://github.com/vidiko/alveus/tree/main"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        "[a](..)",
        ["https://github.com/vidiko/alveus/tree/main"],
        [],
        0,
    ),
    (
        "index.md",
        "edit/main/docs/",
        "[a](../docs/plan/README.md#h)",
        ["plan/README.md#h"],
        [],
        0,
    ),
    (
        "plan/README.md",
        "edit/main/docs/",
        "[a](../../docs/index.md)",
        ["../index.md"],
        [],
        0,
    ),
    ("index.md", "edit/main/docs/", "[a](../docsx/y.md)", [f"{B}docsx/y.md"], [], 0),
    ("index.md", "edit/main/docs/", "[a](../../outside/)", ["../../outside/"], [], 1),
    (
        "plan/README.md",
        "edit/main/docs/",
        "[a](../../CLEANROOM.md)",
        [f"{B}CLEANROOM.md"],
        [],
        0,
    ),
    ("index.md", "", "[a](../CLEANROOM.md)", [f"{H}CLEANROOM.md"], [], 0),
]


class _HtmlCollector(HTMLParser):
    """Collect anchor hrefs, image sources and code text from rendered HTML."""

    def __init__(self) -> None:
        """Start with no collected hrefs, sources or code text."""
        super().__init__(convert_charrefs=True)
        self.hrefs: list[str] = []
        self.srcs: list[str] = []
        self.codes: list[str] = []
        self._code_depth = 0
        self._code_text: list[str] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        """Record a link target, an image source, or the start of a code span."""
        values = dict(attrs)
        if tag == "a":
            self.hrefs.append(values.get("href") or "")
        elif tag == "img":
            self.srcs.append(values.get("src") or "")
        elif tag == "code":
            if self._code_depth == 0:
                self._code_text = []
            self._code_depth += 1

    def handle_endtag(self, tag: str) -> None:
        """Close a code span and store its text once the outermost one ends."""
        if tag == "code" and self._code_depth > 0:
            self._code_depth -= 1
            if self._code_depth == 0:
                self.codes.append("".join(self._code_text).strip())

    def handle_data(self, data: str) -> None:
        """Keep text that sits inside a code span."""
        if self._code_depth > 0:
            self._code_text.append(data)


def _run_case(
    page_uri: str,
    edit_uri: str,
    src: str,
    want_hrefs: list[str],
    want_srcs: list[str],
    want_warns: int,
) -> bool:
    """Render one Markdown snippet through the hook extension and check output."""
    cfg: dict[str, Any] = dict(CFG, edit_uri=edit_uri)
    ext = MOD.RepoLinksExtension(cfg, TRACKED)
    cfg["markdown_extensions"] = [ext]
    page = types.SimpleNamespace(
        file=types.SimpleNamespace(src_uri=page_uri, src_path=page_uri)
    )
    with hooktest.capture_warnings("mkdocs.hooks.repo_links") as handler:
        md_input = MOD.on_page_markdown(src, page, cfg, None)
        # mkdocs.yml loads pymdownx.superfences ahead of the hook; match it.
        md = markdown.Markdown(extensions=["pymdownx.superfences", ext])
        html = md.convert(md_input)
    collector = _HtmlCollector()
    collector.feed(html)
    collector.close()
    got_hrefs = collector.hrefs
    got_srcs = collector.srcs
    got_codes = collector.codes
    code_ok = True if want_hrefs else got_codes == ["[a](../CLEANROOM.md)"]
    ok = (
        got_hrefs == want_hrefs
        and got_srcs == want_srcs
        and len(handler.messages) == want_warns
        and code_ok
    )
    status = "ok  " if ok else "FAIL"
    detail = (
        ""
        if ok
        else f"-> hrefs={got_hrefs!r} srcs={got_srcs!r} warns={len(handler.messages)}"
    )
    print(status, repr(src), detail)
    return ok


def main() -> int:
    """Run all link-rewriting cases and exit 1 on any failure."""
    bad = 0
    for page_uri, edit_uri, src, want_hrefs, want_srcs, want_warns in CASES:
        if not _run_case(page_uri, edit_uri, src, want_hrefs, want_srcs, want_warns):
            bad += 1
    print(f"cases={len(CASES)} failed={bad}")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main())
